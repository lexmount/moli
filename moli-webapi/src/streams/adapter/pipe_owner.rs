//! V8-resident storage codec for one `ReadableStreamPipeTo` operation.
//!
//! Both participating streams retain the same owner object.  The primitive
//! lifecycle is decoded into `moli_streams::pipe::PipeOwnerState`; V8
//! identities and promise residences remain private slots on this object. The
//! object is unique to one pipe operation and is passed directly to callbacks;
//! it is never reset to `Active` or reused for a later pipe.

use super::*;
use moli_streams::pipe::{
    AbortListenerState, PipeLifecycle, PipeOptions, PipeOwnerState, PipeShutdownOperation,
    PipeShutdownSettlements, PipeShutdownStage, PipeTerminalTrigger, PipeTransition,
};
use moli_webapi_declare::WebApiObject;

const PIPE_OWNER_SOURCE_SLOT: &str = "__moliPipeOwnerSource";
const PIPE_OWNER_DESTINATION_SLOT: &str = "__moliPipeOwnerDestination";
const PIPE_OWNER_RESULT_RESOLVER_SLOT: &str = "__moliPipeOwnerResultResolver";
const PIPE_OWNER_LIFECYCLE_SLOT: &str = "__moliPipeOwnerLifecycle";
const PIPE_OWNER_OPTIONS_SLOT: &str = "__moliPipeOwnerOptions";
const PIPE_OWNER_DRAIN_SCHEDULED_SLOT: &str = "__moliPipeOwnerDrainScheduled";
const PIPE_OWNER_READ_PENDING_SLOT: &str = "__moliPipeOwnerReadPending";
const PIPE_OWNER_WRITE_IN_PROGRESS_SLOT: &str = "__moliPipeOwnerWriteInProgress";
const PIPE_OWNER_DRAIN_YIELD_ONCE_SLOT: &str = "__moliPipeOwnerDrainYieldOnce";
const PIPE_OWNER_ABORT_LISTENER_STATE_SLOT: &str = "__moliPipeOwnerAbortListenerState";
const PIPE_OWNER_SHUTDOWN_SETTLEMENTS_SLOT: &str = "__moliPipeOwnerShutdownSettlements";
const PIPE_OWNER_SHUTDOWN_ACTION_ERRORS_SLOT: &str = "__moliPipeOwnerShutdownActionErrors";
const PIPE_OWNER_LAST_WRITE_SLOT: &str = "__moliPipeOwnerLastWrite";
const PIPE_OWNER_SHUTDOWN_REASON_SLOT: &str = "__moliPipeOwnerShutdownReason";
const PIPE_OWNER_ABORT_REGISTRATION_SLOT: &str = "__moliPipeOwnerAbortRegistration";

#[derive(WebApiObject)]
#[webapi(plain)]
struct PipeOwnerDeclaration<'scope> {
    #[webapi(slot = PIPE_OWNER_SOURCE_SLOT)]
    source: v8::Local<'scope, v8::Object>,
    #[webapi(slot = PIPE_OWNER_DESTINATION_SLOT)]
    destination: v8::Local<'scope, v8::Object>,
    #[webapi(slot = PIPE_OWNER_RESULT_RESOLVER_SLOT)]
    result_resolver: v8::Local<'scope, v8::Object>,
    #[webapi(slot = PIPE_OWNER_LIFECYCLE_SLOT, init = 0)]
    lifecycle: (),
    #[webapi(slot = PIPE_OWNER_OPTIONS_SLOT)]
    options: f64,
    #[webapi(slot = PIPE_OWNER_DRAIN_SCHEDULED_SLOT, init = false)]
    drain_scheduled: (),
    #[webapi(slot = PIPE_OWNER_READ_PENDING_SLOT, init = false)]
    read_pending: (),
    #[webapi(slot = PIPE_OWNER_WRITE_IN_PROGRESS_SLOT, init = false)]
    write_in_progress: (),
    #[webapi(slot = PIPE_OWNER_DRAIN_YIELD_ONCE_SLOT, init = false)]
    drain_yield_once: (),
    #[webapi(slot = PIPE_OWNER_ABORT_LISTENER_STATE_SLOT, init = 0)]
    abort_listener_state: (),
    #[webapi(slot = PIPE_OWNER_SHUTDOWN_SETTLEMENTS_SLOT, init = 0)]
    shutdown_settlements: (),
    #[webapi(slot = PIPE_OWNER_SHUTDOWN_ACTION_ERRORS_SLOT, init = "null")]
    shutdown_action_errors: (),
    #[webapi(slot = PIPE_OWNER_LAST_WRITE_SLOT, init = "null")]
    last_write: (),
    #[webapi(slot = PIPE_OWNER_SHUTDOWN_REASON_SLOT, init = "null")]
    shutdown_reason: (),
    #[webapi(slot = PIPE_OWNER_ABORT_REGISTRATION_SLOT, init = "null")]
    abort_registration: (),
}

#[derive(Clone, Copy)]
pub(super) struct PipeOwner<'scope> {
    object: v8::Local<'scope, v8::Object>,
}

impl<'scope> PipeOwner<'scope> {
    pub(super) fn new(
        scope: &mut v8::PinScope<'scope, '_>,
        source: v8::Local<'scope, v8::Object>,
        destination: v8::Local<'scope, v8::Object>,
        result_resolver: v8::Local<'scope, v8::Object>,
        options: PipeOptions,
    ) -> Self {
        let object = PipeOwnerDeclaration::new(
            source,
            destination,
            result_resolver,
            f64::from(options.bits()),
        )
        .bind(scope)
        .expect("PipeOwner declaration should bind");
        Self { object }
    }

    pub(super) const fn object(self) -> v8::Local<'scope, v8::Object> {
        self.object
    }

    pub(super) fn from_source(
        scope: &mut v8::PinScope<'scope, '_>,
        source: v8::Local<'scope, v8::Object>,
    ) -> Option<Self> {
        let object = stream_slot_object(scope, source, READABLE_STREAM_PIPE_OWNER_SLOT)?;
        if object.is_null_or_undefined() {
            return None;
        }
        Some(Self { object })
    }

    pub(super) fn from_value(value: v8::Local<'scope, v8::Value>) -> Option<Self> {
        let object = v8::Local::<v8::Object>::try_from(value).ok()?;
        if object.is_null_or_undefined() {
            return None;
        }
        Some(Self { object })
    }

    pub(super) fn source(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
    ) -> v8::Local<'scope, v8::Object> {
        required_object(scope, self.object, PIPE_OWNER_SOURCE_SLOT, "source")
    }

    pub(super) fn destination(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
    ) -> v8::Local<'scope, v8::Object> {
        required_object(
            scope,
            self.object,
            PIPE_OWNER_DESTINATION_SLOT,
            "destination",
        )
    }

    pub(super) fn result_resolver(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
    ) -> v8::Local<'scope, v8::Object> {
        required_object(
            scope,
            self.object,
            PIPE_OWNER_RESULT_RESOLVER_SLOT,
            "result resolver",
        )
    }

    pub(super) fn state(self, scope: &mut v8::PinScope<'scope, '_>) -> PipeOwnerState {
        PipeOwnerState::from_storage(
            decode_lifecycle(required_u32(scope, self.object, PIPE_OWNER_LIFECYCLE_SLOT)),
            PipeOptions::from_bits(required_u8(scope, self.object, PIPE_OWNER_OPTIONS_SLOT))
                .expect("PipeOwner options slot must contain only defined bits"),
            required_bool(scope, self.object, PIPE_OWNER_DRAIN_SCHEDULED_SLOT),
            required_bool(scope, self.object, PIPE_OWNER_READ_PENDING_SLOT),
            required_bool(scope, self.object, PIPE_OWNER_WRITE_IN_PROGRESS_SLOT),
            required_bool(scope, self.object, PIPE_OWNER_DRAIN_YIELD_ONCE_SLOT),
            decode_abort_listener(required_u32(
                scope,
                self.object,
                PIPE_OWNER_ABORT_LISTENER_STATE_SLOT,
            )),
            PipeShutdownSettlements::from_bits(required_u8(
                scope,
                self.object,
                PIPE_OWNER_SHUTDOWN_SETTLEMENTS_SLOT,
            ))
            .expect("PipeOwner shutdown settlements must contain valid operation outcomes"),
        )
        .expect("PipeOwner storage must contain a valid lifecycle state")
    }

    /// Commit a core transition without invoking JavaScript.
    pub(super) fn apply<C: Copy>(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
        transition: PipeTransition<C>,
    ) {
        let live = self.state(scope);
        assert_eq!(
            live,
            transition.source(),
            "PipeOwner transition source must match live owner state"
        );
        let next = transition.next();
        if live == next {
            return;
        }
        if live.lifecycle() != next.lifecycle() {
            set_number(
                scope,
                self.object,
                PIPE_OWNER_LIFECYCLE_SLOT,
                encode_lifecycle(next.lifecycle()),
            );
        }
        if live.drain_scheduled() != next.drain_scheduled() {
            set_required_bool(
                scope,
                self.object,
                PIPE_OWNER_DRAIN_SCHEDULED_SLOT,
                next.drain_scheduled(),
            );
        }
        if live.read_pending() != next.read_pending() {
            set_required_bool(
                scope,
                self.object,
                PIPE_OWNER_READ_PENDING_SLOT,
                next.read_pending(),
            );
        }
        if live.write_in_progress() != next.write_in_progress() {
            set_required_bool(
                scope,
                self.object,
                PIPE_OWNER_WRITE_IN_PROGRESS_SLOT,
                next.write_in_progress(),
            );
        }
        if live.drain_yield_once() != next.drain_yield_once() {
            set_required_bool(
                scope,
                self.object,
                PIPE_OWNER_DRAIN_YIELD_ONCE_SLOT,
                next.drain_yield_once(),
            );
        }
        if live.abort_listener() != next.abort_listener() {
            set_number(
                scope,
                self.object,
                PIPE_OWNER_ABORT_LISTENER_STATE_SLOT,
                encode_abort_listener(next.abort_listener()),
            );
        }
        if live.shutdown_settlements() != next.shutdown_settlements() {
            set_number(
                scope,
                self.object,
                PIPE_OWNER_SHUTDOWN_SETTLEMENTS_SLOT,
                u32::from(next.shutdown_settlements().bits()),
            );
        }
    }

    pub(super) fn set_shutdown_action_error(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
        operation: PipeShutdownOperation,
        error: v8::Local<'scope, v8::Value>,
    ) {
        let errors = stream_slot_array(scope, self.object, PIPE_OWNER_SHUTDOWN_ACTION_ERRORS_SLOT)
            .unwrap_or_else(|| {
                let errors = v8::Array::new(scope, 2);
                set_required_value(
                    scope,
                    self.object,
                    PIPE_OWNER_SHUTDOWN_ACTION_ERRORS_SLOT,
                    errors.into(),
                );
                errors
            });
        errors
            .set_index(scope, shutdown_operation_index(operation), error)
            .expect("PipeOwner shutdown action error must publish");
    }

    pub(super) fn shutdown_action_error(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
        operation: PipeShutdownOperation,
    ) -> Option<v8::Local<'scope, v8::Value>> {
        stream_slot_array(scope, self.object, PIPE_OWNER_SHUTDOWN_ACTION_ERRORS_SLOT)?
            .get_index(scope, shutdown_operation_index(operation))
    }

    pub(super) fn set_last_write(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
        promise: v8::Local<'scope, v8::Promise>,
    ) {
        set_required_value(
            scope,
            self.object,
            PIPE_OWNER_LAST_WRITE_SLOT,
            promise.into(),
        );
    }

    pub(super) fn last_write(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
    ) -> Option<v8::Local<'scope, v8::Promise>> {
        stream_slot_value(scope, self.object, PIPE_OWNER_LAST_WRITE_SLOT)
            .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
    }

    pub(super) fn set_shutdown_reason(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
        reason: v8::Local<'scope, v8::Value>,
    ) {
        // `undefined` is a valid rejection reason, while the shared private
        // slot helper treats it as an absent slot. Keep every reason inside a
        // residence entry so the full JavaScript value domain is preserved.
        let entry = v8::Array::new(scope, 1);
        entry
            .set_index(scope, 0, reason)
            .expect("PipeOwner shutdown reason must publish");
        set_required_value(
            scope,
            self.object,
            PIPE_OWNER_SHUTDOWN_REASON_SLOT,
            entry.into(),
        );
    }

    pub(super) fn shutdown_reason(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
    ) -> Option<v8::Local<'scope, v8::Value>> {
        stream_slot_value(scope, self.object, PIPE_OWNER_SHUTDOWN_REASON_SLOT)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            .map(|entry| {
                entry
                    .get_index(scope, 0)
                    .expect("PipeOwner shutdown reason entry must retain its value")
            })
    }

    pub(super) fn set_abort_registration(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
        signal: v8::Local<'scope, v8::Object>,
        listener: v8::Local<'scope, v8::Function>,
    ) {
        let registration = v8::Array::new(scope, 2);
        registration
            .set_index(scope, 0, signal.into())
            .expect("PipeOwner AbortSignal must publish");
        registration
            .set_index(scope, 1, listener.into())
            .expect("PipeOwner abort listener must publish");
        set_required_value(
            scope,
            self.object,
            PIPE_OWNER_ABORT_REGISTRATION_SLOT,
            registration.into(),
        );
    }

    pub(super) fn abort_registration(
        self,
        scope: &mut v8::PinScope<'scope, '_>,
    ) -> Option<(
        v8::Local<'scope, v8::Object>,
        v8::Local<'scope, v8::Function>,
    )> {
        let registration =
            stream_slot_array(scope, self.object, PIPE_OWNER_ABORT_REGISTRATION_SLOT)?;
        let signal = registration
            .get_index(scope, 0)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
        let listener = registration
            .get_index(scope, 1)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
        Some((signal, listener))
    }

    pub(super) fn clear_abort_registration(self, scope: &mut v8::PinScope<'scope, '_>) {
        set_required_value(
            scope,
            self.object,
            PIPE_OWNER_ABORT_REGISTRATION_SLOT,
            v8::null(scope).into(),
        );
    }

    pub(super) fn clear_residences(self, scope: &mut v8::PinScope<'scope, '_>) {
        for slot in [
            PIPE_OWNER_SOURCE_SLOT,
            PIPE_OWNER_DESTINATION_SLOT,
            PIPE_OWNER_RESULT_RESOLVER_SLOT,
            PIPE_OWNER_LAST_WRITE_SLOT,
            PIPE_OWNER_SHUTDOWN_REASON_SLOT,
            PIPE_OWNER_SHUTDOWN_ACTION_ERRORS_SLOT,
            PIPE_OWNER_ABORT_REGISTRATION_SLOT,
        ] {
            set_required_value(scope, self.object, slot, v8::null(scope).into());
        }
    }
}

fn required_object<'scope>(
    scope: &mut v8::PinScope<'scope, '_>,
    object: v8::Local<'scope, v8::Object>,
    slot: &'static str,
    label: &'static str,
) -> v8::Local<'scope, v8::Object> {
    stream_slot_object(scope, object, slot)
        .filter(|value| !value.is_null_or_undefined())
        .unwrap_or_else(|| panic!("PipeOwner is missing its {label} residence"))
}

fn required_bool<'scope>(
    scope: &mut v8::PinScope<'scope, '_>,
    object: v8::Local<'scope, v8::Object>,
    slot: &'static str,
) -> bool {
    stream_slot_bool(scope, object, slot)
        .unwrap_or_else(|| panic!("PipeOwner is missing boolean slot {slot}"))
}

fn required_u8<'scope>(
    scope: &mut v8::PinScope<'scope, '_>,
    object: v8::Local<'scope, v8::Object>,
    slot: &'static str,
) -> u8 {
    let value = required_u32(scope, object, slot);
    u8::try_from(value).unwrap_or_else(|_| panic!("PipeOwner slot {slot} exceeds u8"))
}

fn required_u32<'scope>(
    scope: &mut v8::PinScope<'scope, '_>,
    object: v8::Local<'scope, v8::Object>,
    slot: &'static str,
) -> u32 {
    let value = stream_slot_number(scope, object, slot)
        .unwrap_or_else(|| panic!("PipeOwner is missing numeric slot {slot}"));
    assert!(
        value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= f64::from(u32::MAX),
        "PipeOwner slot {slot} has invalid numeric value {value}"
    );
    value as u32
}

fn set_number<'scope>(
    scope: &mut v8::PinScope<'scope, '_>,
    object: v8::Local<'scope, v8::Object>,
    slot: &'static str,
    value: u32,
) {
    let value = v8::Number::new(scope, f64::from(value));
    set_required_value(scope, object, slot, value.into());
}

fn set_required_bool<'scope>(
    scope: &mut v8::PinScope<'scope, '_>,
    object: v8::Local<'scope, v8::Object>,
    slot: &'static str,
    value: bool,
) {
    let value = v8::Boolean::new(scope, value);
    set_required_value(scope, object, slot, value.into());
}

fn set_required_value<'scope>(
    scope: &mut v8::PinScope<'scope, '_>,
    object: v8::Local<'scope, v8::Object>,
    slot: &'static str,
    value: v8::Local<'scope, v8::Value>,
) {
    set_required_stream_slot_value(scope, object, slot, value, slot);
}

const fn shutdown_operation_index(operation: PipeShutdownOperation) -> u32 {
    match operation {
        PipeShutdownOperation::Destination => 0,
        PipeShutdownOperation::Source => 1,
    }
}

const PIPE_LIFECYCLE_ACTIVE: u32 = 0;
const PIPE_LIFECYCLE_FINISHED: u32 = 1;
const PIPE_LIFECYCLE_SHUTDOWN_BASE: u32 = 2;
const PIPE_TERMINAL_TRIGGER_COUNT: u32 = 5;

const fn encode_lifecycle(value: PipeLifecycle) -> u32 {
    match value {
        PipeLifecycle::Active => PIPE_LIFECYCLE_ACTIVE,
        PipeLifecycle::Finished => PIPE_LIFECYCLE_FINISHED,
        PipeLifecycle::ShuttingDown { stage, trigger } => {
            PIPE_LIFECYCLE_SHUTDOWN_BASE
                + encode_shutdown_stage(stage) * PIPE_TERMINAL_TRIGGER_COUNT
                + encode_terminal_trigger(trigger)
        }
    }
}

fn decode_lifecycle(value: u32) -> PipeLifecycle {
    match value {
        PIPE_LIFECYCLE_ACTIVE => PipeLifecycle::Active,
        PIPE_LIFECYCLE_FINISHED => PipeLifecycle::Finished,
        value => {
            let encoded = value
                .checked_sub(PIPE_LIFECYCLE_SHUTDOWN_BASE)
                .unwrap_or_else(|| panic!("PipeOwner has invalid lifecycle {value}"));
            let stage = decode_shutdown_stage(encoded / PIPE_TERMINAL_TRIGGER_COUNT);
            let trigger = decode_terminal_trigger(encoded % PIPE_TERMINAL_TRIGGER_COUNT);
            PipeLifecycle::ShuttingDown { stage, trigger }
        }
    }
}

const fn encode_abort_listener(value: AbortListenerState) -> u32 {
    match value {
        AbortListenerState::None => 0,
        AbortListenerState::Registered => 1,
        AbortListenerState::Aborting => 2,
    }
}

fn decode_abort_listener(value: u32) -> AbortListenerState {
    match value {
        0 => AbortListenerState::None,
        1 => AbortListenerState::Registered,
        2 => AbortListenerState::Aborting,
        _ => panic!("PipeOwner has invalid abort-listener state {value}"),
    }
}

const fn encode_shutdown_stage(value: PipeShutdownStage) -> u32 {
    match value {
        PipeShutdownStage::WaitingForWritePublication => 0,
        PipeShutdownStage::WaitingForLastWrite => 1,
        PipeShutdownStage::RunningActions => 2,
        PipeShutdownStage::Finalizing => 3,
    }
}

fn decode_shutdown_stage(value: u32) -> PipeShutdownStage {
    match value {
        0 => PipeShutdownStage::WaitingForWritePublication,
        1 => PipeShutdownStage::WaitingForLastWrite,
        2 => PipeShutdownStage::RunningActions,
        3 => PipeShutdownStage::Finalizing,
        _ => panic!("PipeOwner has invalid shutdown stage {value}"),
    }
}

const fn encode_terminal_trigger(value: PipeTerminalTrigger) -> u32 {
    match value {
        PipeTerminalTrigger::SourceErrored => 0,
        PipeTerminalTrigger::DestinationErrored => 1,
        PipeTerminalTrigger::SourceClosed => 2,
        PipeTerminalTrigger::DestinationClosed => 3,
        PipeTerminalTrigger::Aborted => 4,
    }
}

fn decode_terminal_trigger(value: u32) -> PipeTerminalTrigger {
    match value {
        0 => PipeTerminalTrigger::SourceErrored,
        1 => PipeTerminalTrigger::DestinationErrored,
        2 => PipeTerminalTrigger::SourceClosed,
        3 => PipeTerminalTrigger::DestinationClosed,
        4 => PipeTerminalTrigger::Aborted,
        _ => panic!("PipeOwner has invalid terminal trigger {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_lifecycle_codec_round_trips_every_legal_state() {
        for lifecycle in [PipeLifecycle::Active, PipeLifecycle::Finished] {
            assert_eq!(decode_lifecycle(encode_lifecycle(lifecycle)), lifecycle);
        }
        for stage in [
            PipeShutdownStage::WaitingForWritePublication,
            PipeShutdownStage::WaitingForLastWrite,
            PipeShutdownStage::RunningActions,
            PipeShutdownStage::Finalizing,
        ] {
            for trigger in [
                PipeTerminalTrigger::SourceErrored,
                PipeTerminalTrigger::DestinationErrored,
                PipeTerminalTrigger::SourceClosed,
                PipeTerminalTrigger::DestinationClosed,
                PipeTerminalTrigger::Aborted,
            ] {
                let lifecycle = PipeLifecycle::ShuttingDown { stage, trigger };
                assert_eq!(decode_lifecycle(encode_lifecycle(lifecycle)), lifecycle);
            }
        }
    }
}
