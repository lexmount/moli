use super::*;
use crate::util::{define_v8_array_data_property, set_null_prototype, v8str};
use crate::web_api_interfaces;
use crate::webidl;
use moli_streams::readable::byte_controller::{
    ArrayBufferViewKind, ByobRequestPlan, ByteControllerSnapshot, ByteEnqueueContinuationPlan,
    ByteEnqueueStartPlan, ByteReadIntoPlan, ClosedByobStepPlan, ClosedReleasedHeadPlan,
    DefaultReadStepPlan, DescriptorResultPlan, FinishByteClosePlan, FlushReleasedHeadPlan,
    HeadResolutionError, PullIntoDescriptor as PullIntoDescriptorState, PullIntoReaderType,
    ReadIntoError, ReleaseReaderPlan, ReplacementView, ReplacementViewError, RespondAction,
    RespondError, RespondPlan,
};
use moli_webapi_declare::WebApiObject;

const DESCRIPTOR_BUFFER_INDEX: u32 = 0;
const DESCRIPTOR_BUFFER_BYTE_LENGTH_INDEX: u32 = 1;
const DESCRIPTOR_BYTE_OFFSET_INDEX: u32 = 2;
const DESCRIPTOR_BYTE_LENGTH_INDEX: u32 = 3;
const DESCRIPTOR_BYTES_FILLED_INDEX: u32 = 4;
const DESCRIPTOR_MINIMUM_FILL_INDEX: u32 = 5;
const DESCRIPTOR_ELEMENT_SIZE_INDEX: u32 = 6;
const DESCRIPTOR_VIEW_KIND_INDEX: u32 = 7;
const DESCRIPTOR_READER_TYPE_INDEX: u32 = 8;

fn array_buffer_view_kind(value: v8::Local<'_, v8::Value>) -> Option<ArrayBufferViewKind> {
    if value.is_data_view() {
        Some(ArrayBufferViewKind::DataView)
    } else if value.is_int8_array() {
        Some(ArrayBufferViewKind::Int8)
    } else if value.is_uint8_array() {
        Some(ArrayBufferViewKind::Uint8)
    } else if value.is_uint8_clamped_array() {
        Some(ArrayBufferViewKind::Uint8Clamped)
    } else if value.is_int16_array() {
        Some(ArrayBufferViewKind::Int16)
    } else if value.is_uint16_array() {
        Some(ArrayBufferViewKind::Uint16)
    } else if value.is_int32_array() {
        Some(ArrayBufferViewKind::Int32)
    } else if value.is_uint32_array() {
        Some(ArrayBufferViewKind::Uint32)
    } else if value.is_float16_array() {
        Some(ArrayBufferViewKind::Float16)
    } else if value.is_float32_array() {
        Some(ArrayBufferViewKind::Float32)
    } else if value.is_float64_array() {
        Some(ArrayBufferViewKind::Float64)
    } else if value.is_big_int64_array() {
        Some(ArrayBufferViewKind::BigInt64)
    } else if value.is_big_uint64_array() {
        Some(ArrayBufferViewKind::BigUint64)
    } else {
        None
    }
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ReadableStreamBYOBRequest)]
struct ReadableStreamByobRequestObjectDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_BYOB_REQUEST_CONTROLLER_SLOT)]
    controller: v8::Local<'scope, v8::Object>,
    #[webapi(slot = READABLE_STREAM_BYOB_REQUEST_VIEW_SLOT)]
    view: v8::Local<'scope, v8::Uint8Array>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ReadableStreamBYOBRequest.respond")]
struct ReadableStreamByobRequestRespondArgs {
    #[webidl(required, converter = "enforce_range_unsigned_long_long")]
    bytes_written: u64,
}

struct TransferredView<'s> {
    buffer: v8::Local<'s, v8::ArrayBuffer>,
    buffer_byte_length: usize,
    byte_offset: usize,
    byte_length: usize,
    kind: ArrayBufferViewKind,
}

#[derive(Clone, Copy)]
struct PullIntoDescriptor<'s> {
    storage: v8::Local<'s, v8::Array>,
    buffer: v8::Local<'s, v8::ArrayBuffer>,
    buffer_byte_length: usize,
    byte_offset: usize,
    byte_length: usize,
    bytes_filled: usize,
    minimum_fill: usize,
    element_size: usize,
    view_kind: ArrayBufferViewKind,
    reader_type: PullIntoReaderType,
}

/// One ephemeral decode of the adapter-owned byte-controller storage.
///
/// The pending-descriptor array and its head storage object are identity
/// tokens, just like the readable queue array in `queue.rs`. Plans may carry
/// pure descriptor metadata into the core, but a commit is accepted only if
/// these exact V8 objects still own the live head.
struct ReadableByteControllerAdapterSnapshot<'s> {
    controller: ByteControllerSnapshot,
    pending_pull_intos: Option<v8::Local<'s, v8::Array>>,
    head_storage: Option<v8::Local<'s, v8::Array>>,
    cached_byob_request: Option<v8::Local<'s, v8::Object>>,
}

impl<'s> ReadableByteControllerAdapterSnapshot<'s> {
    fn controller(&self) -> &ByteControllerSnapshot {
        &self.controller
    }

    fn live_head(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        stream: v8::Local<'s, v8::Object>,
        expected: PullIntoDescriptorState,
    ) -> Option<PullIntoDescriptor<'s>> {
        let expected_pending = self.pending_pull_intos?;
        let expected_storage = self.head_storage?;
        let current_pending = pending_pull_intos(scope, stream)?;
        if !current_pending.strict_equals(expected_pending.into()) {
            return None;
        }
        let current_storage = current_pending
            .get_index(scope, 0)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
        if !current_storage.strict_equals(expected_storage.into()) {
            return None;
        }
        parse_descriptor(scope, current_storage)
            .filter(|descriptor| descriptor.state() == Some(expected))
    }

    fn live_cached_byob_request(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        stream: v8::Local<'s, v8::Object>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let expected = self.cached_byob_request?;
        let current = stream_slot_object(scope, stream, READABLE_BYTE_STREAM_BYOB_REQUEST_SLOT)?;
        current.strict_equals(expected.into()).then_some(current)
    }
}

impl PullIntoDescriptor<'_> {
    fn state(self) -> Option<PullIntoDescriptorState> {
        if self.element_size != self.view_kind.element_size() {
            return None;
        }
        PullIntoDescriptorState::new(
            self.buffer_byte_length,
            self.byte_offset,
            self.byte_length,
            self.bytes_filled,
            self.minimum_fill,
            self.view_kind,
            self.reader_type,
        )
        .ok()
    }
}

#[derive(Clone, Copy)]
struct DescriptorResolution<'s> {
    descriptor: PullIntoDescriptor<'s>,
    pending: v8::Local<'s, v8::Object>,
    done: bool,
}

#[derive(Clone, Copy)]
struct PreparedDescriptorResolution<'s> {
    pending: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    done: bool,
}

#[derive(Clone, Copy)]
struct DefaultReadResolution<'s> {
    pending: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
}

struct PreparedByteReadStart<'s> {
    controller_snapshot: ReadableByteControllerAdapterSnapshot<'s>,
    transferred: TransferredView<'s>,
    read_state: PullIntoDescriptorState,
    read_plan: ByteReadIntoPlan,
}

fn byte_stream_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &'static str,
) -> v8::Local<'s, v8::Value> {
    v8::Exception::type_error(scope, v8str(scope, message))
}

fn byte_stream_range_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &'static str,
) -> v8::Local<'s, v8::Value> {
    v8::Exception::range_error(scope, v8str(scope, message))
}

fn internal_array<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, 0);
    set_null_prototype(scope, array.into());
    array
}

pub(in crate::context_bootstrap) fn initialize_readable_byte_stream_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    auto_allocate_chunk_size: Option<u64>,
) {
    let pending_pull_intos = internal_array(scope);
    set_stream_slot_value(
        scope,
        stream,
        READABLE_BYTE_STREAM_PENDING_PULL_INTOS_SLOT,
        pending_pull_intos.into(),
    );
    set_stream_slot_value(
        scope,
        stream,
        READABLE_BYTE_STREAM_BYOB_REQUEST_SLOT,
        v8::null(scope).into(),
    );
    let auto_allocate_chunk_size = auto_allocate_chunk_size
        .map(|value| v8::Number::new(scope, value as f64).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_stream_slot_value(
        scope,
        stream,
        READABLE_BYTE_STREAM_AUTO_ALLOCATE_CHUNK_SIZE_SLOT,
        auto_allocate_chunk_size,
    );
}

fn pending_pull_intos<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    stream_slot_array(scope, stream, READABLE_BYTE_STREAM_PENDING_PULL_INTOS_SLOT)
}

fn append_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    descriptor: v8::Local<'s, v8::Array>,
) -> bool {
    let Some(pending) = pending_pull_intos(scope, stream) else {
        return false;
    };
    define_v8_array_data_property(scope, pending, pending.length(), descriptor.into()).is_some()
}

fn head_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<PullIntoDescriptor<'s>> {
    let pending = pending_pull_intos(scope, stream)?;
    let storage = pending
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
    parse_descriptor(scope, storage)
}

fn pending_descriptor_states<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pending: Option<v8::Local<'s, v8::Array>>,
) -> Vec<Option<PullIntoDescriptorState>> {
    let Some(pending) = pending else {
        return Vec::new();
    };
    (0..pending.length())
        .map(|index| {
            pending
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
                .and_then(|storage| parse_descriptor(scope, storage))
                .and_then(PullIntoDescriptor::state)
        })
        .collect()
}

fn readable_byte_controller_adapter_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> ReadableByteControllerAdapterSnapshot<'s> {
    let pending_pull_intos = pending_pull_intos(scope, stream);
    let head_storage = pending_pull_intos.and_then(|pending| {
        pending
            .get_index(scope, 0)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    });
    let cached_byob_request =
        stream_slot_object(scope, stream, READABLE_BYTE_STREAM_BYOB_REQUEST_SLOT);
    let auto_allocate_chunk_size = stream_slot_number(
        scope,
        stream,
        READABLE_BYTE_STREAM_AUTO_ALLOCATE_CHUNK_SIZE_SLOT,
    )
    .map(|size| size as usize);
    let pipe_read_pending = super::pipe::pipe_owner_state_for_source(scope, stream)
        .map(moli_streams::pipe::PipeOwnerState::read_pending);
    let controller = ByteControllerSnapshot::new(
        readable_stream_snapshot(scope, stream),
        pending_descriptor_states(scope, pending_pull_intos),
        cached_byob_request.is_some(),
        auto_allocate_chunk_size,
        pipe_read_pending,
    );
    ReadableByteControllerAdapterSnapshot {
        controller,
        pending_pull_intos,
        head_storage,
        cached_byob_request,
    }
}

fn readable_byte_controller_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> ByteControllerSnapshot {
    readable_byte_controller_adapter_snapshot(scope, stream).controller
}

fn shift_head_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(pending) = pending_pull_intos(scope, stream) else {
        return false;
    };
    if pending.length() == 0 {
        return false;
    }
    let next = internal_array(scope);
    for index in 1..pending.length() {
        let Some(value) = pending.get_index(scope, index) else {
            return false;
        };
        if define_v8_array_data_property(scope, next, next.length(), value).is_none() {
            return false;
        }
    }
    set_stream_slot_value(
        scope,
        stream,
        READABLE_BYTE_STREAM_PENDING_PULL_INTOS_SLOT,
        next.into(),
    );
    true
}

fn clear_descriptors<'s>(scope: &mut v8::PinScope<'s, '_>, stream: v8::Local<'s, v8::Object>) {
    invalidate_byob_request(scope, stream);
    let pending = internal_array(scope);
    set_stream_slot_value(
        scope,
        stream,
        READABLE_BYTE_STREAM_PENDING_PULL_INTOS_SLOT,
        pending.into(),
    );
}

fn number_field(
    scope: &mut v8::PinScope<'_, '_>,
    descriptor: v8::Local<'_, v8::Array>,
    index: u32,
) -> Option<usize> {
    descriptor
        .get_index(scope, index)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as usize)
}

fn parse_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    storage: v8::Local<'s, v8::Array>,
) -> Option<PullIntoDescriptor<'s>> {
    let buffer = storage
        .get_index(scope, DESCRIPTOR_BUFFER_INDEX)
        .and_then(|value| v8::Local::<v8::ArrayBuffer>::try_from(value).ok())?;
    let view_kind = storage
        .get_index(scope, DESCRIPTOR_VIEW_KIND_INDEX)
        .and_then(|value| value.number_value(scope))
        .and_then(|value| ArrayBufferViewKind::from_discriminant(value as u32))?;
    let reader_type = PullIntoReaderType::from_discriminant(number_field(
        scope,
        storage,
        DESCRIPTOR_READER_TYPE_INDEX,
    )? as u32)?;
    let descriptor = PullIntoDescriptor {
        storage,
        buffer,
        buffer_byte_length: number_field(scope, storage, DESCRIPTOR_BUFFER_BYTE_LENGTH_INDEX)?,
        byte_offset: number_field(scope, storage, DESCRIPTOR_BYTE_OFFSET_INDEX)?,
        byte_length: number_field(scope, storage, DESCRIPTOR_BYTE_LENGTH_INDEX)?,
        bytes_filled: number_field(scope, storage, DESCRIPTOR_BYTES_FILLED_INDEX)?,
        minimum_fill: number_field(scope, storage, DESCRIPTOR_MINIMUM_FILL_INDEX)?,
        element_size: number_field(scope, storage, DESCRIPTOR_ELEMENT_SIZE_INDEX)?,
        view_kind,
        reader_type,
    };
    descriptor.state()?;
    Some(descriptor)
}

fn set_descriptor_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    descriptor: v8::Local<'s, v8::Array>,
    index: u32,
    value: usize,
) -> bool {
    let value = v8::Number::new(scope, value as f64);
    define_v8_array_data_property(scope, descriptor, index, value.into()).is_some()
}

fn set_descriptor_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    descriptor: v8::Local<'s, v8::Array>,
    buffer: v8::Local<'s, v8::ArrayBuffer>,
) -> bool {
    define_v8_array_data_property(scope, descriptor, DESCRIPTOR_BUFFER_INDEX, buffer.into())
        .is_some()
}

fn new_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transferred: TransferredView<'s>,
    minimum_fill: usize,
    reader_type: PullIntoReaderType,
) -> Option<PullIntoDescriptor<'s>> {
    let storage = internal_array(scope);
    let fields: [(u32, v8::Local<'s, v8::Value>); 9] = [
        (DESCRIPTOR_BUFFER_INDEX, transferred.buffer.into()),
        (
            DESCRIPTOR_BUFFER_BYTE_LENGTH_INDEX,
            v8::Number::new(scope, transferred.buffer_byte_length as f64).into(),
        ),
        (
            DESCRIPTOR_BYTE_OFFSET_INDEX,
            v8::Number::new(scope, transferred.byte_offset as f64).into(),
        ),
        (
            DESCRIPTOR_BYTE_LENGTH_INDEX,
            v8::Number::new(scope, transferred.byte_length as f64).into(),
        ),
        (
            DESCRIPTOR_BYTES_FILLED_INDEX,
            v8::Number::new(scope, 0.0).into(),
        ),
        (
            DESCRIPTOR_MINIMUM_FILL_INDEX,
            v8::Number::new(scope, minimum_fill as f64).into(),
        ),
        (
            DESCRIPTOR_ELEMENT_SIZE_INDEX,
            v8::Number::new(scope, transferred.kind.element_size() as f64).into(),
        ),
        (
            DESCRIPTOR_VIEW_KIND_INDEX,
            v8::Number::new(scope, transferred.kind as u32 as f64).into(),
        ),
        (
            DESCRIPTOR_READER_TYPE_INDEX,
            v8::Number::new(scope, reader_type as u32 as f64).into(),
        ),
    ];
    for (index, value) in fields {
        define_v8_array_data_property(scope, storage, index, value)?;
    }
    parse_descriptor(scope, storage)
}

fn transfer_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<TransferredView<'s>, v8::Local<'s, v8::Value>> {
    let view = v8::Local::<v8::ArrayBufferView>::try_from(value).map_err(|_| {
        byte_stream_type_error(
            scope,
            "A readable byte stream operation requires an ArrayBufferView",
        )
    })?;
    let kind = array_buffer_view_kind(value).ok_or_else(|| {
        byte_stream_type_error(
            scope,
            "A readable byte stream operation requires an ArrayBufferView",
        )
    })?;
    let byte_offset = view.byte_offset();
    let byte_length = view.byte_length();
    let buffer = view.buffer(scope).ok_or_else(|| {
        byte_stream_type_error(
            scope,
            "The ArrayBufferView must use an ArrayBuffer backing store",
        )
    })?;
    if buffer.was_detached() || !buffer.is_detachable() {
        return Err(byte_stream_type_error(
            scope,
            "The ArrayBufferView backing buffer could not be transferred",
        ));
    }
    let buffer_byte_length = buffer.byte_length();
    let backing_store = buffer.get_backing_store();
    if backing_store.is_shared() || backing_store.is_resizable_by_user_javascript() {
        return Err(byte_stream_type_error(
            scope,
            "Shared or resizable ArrayBuffer views cannot be transferred to a readable byte stream",
        ));
    }
    let transferred_buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    if buffer.detach(None) != Some(true) {
        return Err(byte_stream_type_error(
            scope,
            "The ArrayBufferView backing buffer could not be transferred",
        ));
    }
    Ok(TransferredView {
        buffer: transferred_buffer,
        buffer_byte_length,
        byte_offset,
        byte_length,
        kind,
    })
}

fn transfer_descriptor_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    descriptor: &mut PullIntoDescriptor<'s>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    let backing_store = descriptor.buffer.get_backing_store();
    if descriptor.buffer.was_detached()
        || !descriptor.buffer.is_detachable()
        || backing_store.is_shared()
    {
        return Err(byte_stream_type_error(
            scope,
            "The BYOB request backing buffer could not be transferred",
        ));
    }
    let transferred = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    if !set_descriptor_buffer(scope, descriptor.storage, transferred) {
        return Err(byte_stream_type_error(
            scope,
            "The BYOB request backing buffer could not be transferred",
        ));
    }
    if descriptor.buffer.detach(None) != Some(true) {
        let _ = set_descriptor_buffer(scope, descriptor.storage, descriptor.buffer);
        return Err(byte_stream_type_error(
            scope,
            "The BYOB request backing buffer could not be transferred",
        ));
    }
    descriptor.buffer = transferred;
    Ok(())
}

fn create_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: ArrayBufferViewKind,
    buffer: v8::Local<'s, v8::ArrayBuffer>,
    byte_offset: usize,
    byte_length: usize,
) -> Option<v8::Local<'s, v8::Value>> {
    if !byte_length.is_multiple_of(kind.element_size()) {
        return None;
    }
    let element_length = byte_length / kind.element_size();
    match kind {
        ArrayBufferViewKind::DataView => {
            Some(v8::DataView::new(scope, buffer, byte_offset, byte_length).into())
        }
        ArrayBufferViewKind::Int8 => {
            v8::Int8Array::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
        ArrayBufferViewKind::Uint8 => {
            v8::Uint8Array::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
        ArrayBufferViewKind::Uint8Clamped => {
            v8::Uint8ClampedArray::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
        ArrayBufferViewKind::Int16 => {
            v8::Int16Array::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
        ArrayBufferViewKind::Uint16 => {
            v8::Uint16Array::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
        ArrayBufferViewKind::Int32 => {
            v8::Int32Array::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
        ArrayBufferViewKind::Uint32 => {
            v8::Uint32Array::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
        ArrayBufferViewKind::Float16 => {
            v8::Float16Array::new(scope, buffer, byte_offset, element_length).map(|view| {
                // `rusty_v8` 146 exposes the Float16Array downcast but omits
                // the otherwise mechanical upcast implemented for every
                // other typed-array class. The V8 inheritance relation makes
                // this the same representation-preserving cast.
                unsafe { v8::Local::<v8::Value>::cast_unchecked(view) }
            })
        }
        ArrayBufferViewKind::Float32 => {
            v8::Float32Array::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
        ArrayBufferViewKind::Float64 => {
            v8::Float64Array::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
        ArrayBufferViewKind::BigInt64 => {
            v8::BigInt64Array::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
        ArrayBufferViewKind::BigUint64 => {
            v8::BigUint64Array::new(scope, buffer, byte_offset, element_length).map(Into::into)
        }
    }
}

fn copy_bytes_into_descriptor(
    descriptor: &PullIntoDescriptor<'_>,
    start: usize,
    bytes: &[u8],
) -> bool {
    let end = match start.checked_add(bytes.len()) {
        Some(value) => value,
        None => return false,
    };
    let backing_store = descriptor.buffer.get_backing_store();
    if end > backing_store.byte_length() {
        return false;
    }
    for (slot, byte) in backing_store[start..end].iter().zip(bytes) {
        slot.set(*byte);
    }
    true
}

fn fill_descriptor_from_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    descriptor: &mut PullIntoDescriptor<'s>,
) -> Result<bool, v8::Local<'s, v8::Value>> {
    let state = descriptor.state().ok_or_else(|| {
        byte_stream_type_error(
            scope,
            "The readable byte stream pull-into descriptor is invalid",
        )
    })?;
    let available = readable_stream_queue_total_size(scope, stream) as usize;
    let plan = state.plan_fill(available);
    let amount = plan.bytes_to_copy();
    if amount == 0 {
        return Ok(plan.ready());
    }
    // Validate the destination before the queue owner commits its replacement
    // generation. No author code runs between this check and the copy, so a
    // failure cannot consume bytes without also storing them in the pull-into
    // buffer.
    let destination = plan.destination();
    if descriptor.buffer.was_detached()
        || destination.end() > descriptor.buffer.get_backing_store().byte_length()
    {
        return Err(byte_stream_type_error(
            scope,
            "The readable byte stream pull-into buffer is invalid",
        ));
    }
    let bytes = take_byte_stream_bytes(scope, stream, amount)
        .map_err(|_| readable_stream_queue_error_value(scope))?
        .ok_or_else(|| readable_stream_queue_error_value(scope))?;
    if bytes.len() != amount
        || !copy_bytes_into_descriptor(descriptor, destination.offset(), &bytes)
    {
        return Err(byte_stream_type_error(
            scope,
            "The readable byte stream pull-into buffer is invalid",
        ));
    }
    descriptor.bytes_filled = plan.descriptor().bytes_filled();
    if !set_descriptor_number(
        scope,
        descriptor.storage,
        DESCRIPTOR_BYTES_FILLED_INDEX,
        descriptor.bytes_filled,
    ) {
        return Err(byte_stream_type_error(
            scope,
            "The readable byte stream pull-into descriptor is invalid",
        ));
    }
    Ok(plan.ready())
}

fn prepare_descriptor_resolution<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolution: DescriptorResolution<'s>,
) -> Result<PreparedDescriptorResolution<'s>, v8::Local<'s, v8::Value>> {
    let DescriptorResolution {
        mut descriptor,
        pending,
        done,
    } = resolution;
    let result_plan = descriptor
        .state()
        .ok_or_else(|| {
            byte_stream_type_error(
                scope,
                "The readable byte stream result descriptor is invalid",
            )
        })?
        .plan_resolution(done)
        .result();
    transfer_descriptor_buffer(scope, &mut descriptor)?;
    let value = match result_plan {
        DescriptorResultPlan::Undefined => v8::undefined(scope).into(),
        DescriptorResultPlan::View { kind, range } => create_view(
            scope,
            kind,
            descriptor.buffer,
            range.offset(),
            range.length(),
        )
        .ok_or_else(|| {
            byte_stream_type_error(scope, "The readable byte stream result view is invalid")
        })?,
    };
    Ok(PreparedDescriptorResolution {
        pending,
        value,
        done,
    })
}

fn resolve_descriptor_batch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolutions: Vec<DescriptorResolution<'s>>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    let mut prepared = Vec::with_capacity(resolutions.len());
    for resolution in resolutions.iter().copied() {
        match prepare_descriptor_resolution(scope, resolution) {
            Ok(resolution) => prepared.push(resolution),
            Err(error) => {
                // The queue and descriptor list have already committed. Reject
                // every request removed by that commit so an internal V8
                // conversion failure can never turn into a hanging read.
                for resolution in resolutions {
                    error_read_request(scope, resolution.pending, error);
                }
                return Err(error);
            }
        }
    }
    // Public promise resolution and internal request steps can both re-enter
    // stream machinery. They are therefore deliberately the final phase,
    // after every affected descriptor, queue entry, and cached BYOB request
    // has committed.
    for resolution in prepared {
        fulfill_read_request(scope, resolution.pending, resolution.value, resolution.done);
    }
    Ok(())
}

fn resolve_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    descriptor: PullIntoDescriptor<'s>,
    pending: v8::Local<'s, v8::Object>,
    done: bool,
) -> Result<(), v8::Local<'s, v8::Value>> {
    resolve_descriptor_batch(
        scope,
        vec![DescriptorResolution {
            descriptor,
            pending,
            done,
        }],
    )
}

fn reject_byte_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let (promise, pending) = new_pending_read_promise(scope)?;
    reject_pending_read(scope, pending, reason);
    Some(promise)
}

pub(in crate::context_bootstrap) fn read_into_byte_stream_as_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    view_value: v8::Local<'s, v8::Value>,
    minimum_elements: usize,
) -> Option<v8::Local<'s, v8::Promise>> {
    let prepared =
        prepare_read_into_byte_stream_as_promise(scope, stream, view_value, minimum_elements)?;
    if prepared.pull_after_attach() {
        maybe_pull_stream(scope, stream);
    }
    Some(prepared.promise())
}

fn reject_prepared_byte_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<PreparedReadableStreamRead<'s>> {
    reject_byte_read(scope, reason).map(|promise| PreparedReadableStreamRead::new(promise, false))
}

/// Creates the public BYOB reader's promise-backed read-into request and
/// commits it without invoking the source pull algorithm.
pub(in crate::context_bootstrap) fn prepare_read_into_byte_stream_as_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    view_value: v8::Local<'s, v8::Value>,
    minimum_elements: usize,
) -> Option<PreparedReadableStreamRead<'s>> {
    let start = match prepare_byte_read_start(scope, stream, view_value, minimum_elements) {
        Ok(start) => start,
        Err(reason) => return reject_prepared_byte_read(scope, reason),
    };
    let (promise, request) = new_pending_read_promise(scope)?;
    let pull_after_attach = commit_byte_read_start(scope, stream, start, request);
    Some(PreparedReadableStreamRead::new(promise, pull_after_attach))
}

/// Performs the BYOB-reader read algorithm with an internal read-into request.
/// The tee adapter is the only current owner; its view comes from an existing
/// branch descriptor, so failure during preflight indicates a broken internal
/// ownership invariant rather than a recoverable author error.
pub(in crate::context_bootstrap) fn perform_read_into_byte_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    view_value: v8::Local<'s, v8::Value>,
    minimum_elements: usize,
    request: v8::Local<'s, v8::Object>,
) -> bool {
    let start = prepare_byte_read_start(scope, stream, view_value, minimum_elements)
        .unwrap_or_else(|_| panic!("internal byte-stream read-into preflight must succeed"));
    commit_byte_read_start(scope, stream, start, request)
}

fn prepare_byte_read_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    view_value: v8::Local<'s, v8::Value>,
    minimum_elements: usize,
) -> Result<PreparedByteReadStart<'s>, v8::Local<'s, v8::Value>> {
    let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(view_value) else {
        return Err(byte_stream_type_error(
            scope,
            "BYOB read requires an ArrayBufferView",
        ));
    };
    let Some(kind) = array_buffer_view_kind(view_value) else {
        return Err(byte_stream_type_error(
            scope,
            "BYOB read requires an ArrayBufferView",
        ));
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(byte_stream_type_error(
            scope,
            "BYOB read requires an ArrayBuffer-backed view",
        ));
    };
    let read_state = match PullIntoDescriptorState::for_read(
        buffer.byte_length(),
        view.byte_offset(),
        view.byte_length(),
        minimum_elements,
        kind,
        PullIntoReaderType::Byob,
    ) {
        Ok(state) => state,
        Err(ReadIntoError::EmptyView) => {
            return Err(byte_stream_type_error(
                scope,
                "BYOB read requires a non-empty view",
            ));
        }
        Err(ReadIntoError::ZeroMinimum) => {
            return Err(byte_stream_type_error(
                scope,
                "BYOB read minimum must be greater than zero",
            ));
        }
        Err(ReadIntoError::MinimumExceedsCapacity) => {
            return Err(byte_stream_range_error(
                scope,
                "BYOB read minimum exceeds the view capacity",
            ));
        }
        Err(ReadIntoError::InvalidBounds) => {
            return Err(byte_stream_type_error(
                scope,
                "BYOB read view has invalid bounds",
            ));
        }
    };
    disturb_readable_stream(scope, stream);
    let controller_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
    let read_plan = controller_snapshot.controller().plan_read_into();
    if read_plan == ByteReadIntoPlan::RejectStoredError
        && let Some(error) = readable_stream_error(scope, stream)
    {
        return Err(error);
    }
    let transferred = transfer_view(scope, view_value)?;
    Ok(PreparedByteReadStart {
        controller_snapshot,
        transferred,
        read_state,
        read_plan,
    })
}

fn commit_byte_read_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    start: PreparedByteReadStart<'s>,
    request: v8::Local<'s, v8::Object>,
) -> bool {
    let PreparedByteReadStart {
        controller_snapshot,
        transferred,
        read_state,
        read_plan,
    } = start;
    let mut descriptor = super::utils::require_internal_stream_value(
        new_descriptor(
            scope,
            transferred,
            read_state.minimum_fill(),
            PullIntoReaderType::Byob,
        ),
        "pull-into descriptor allocation",
        "byte-stream read request",
    );
    match read_plan {
        ByteReadIntoPlan::ResolveDone => {
            let _ = resolve_descriptor(scope, descriptor, request, true);
            return false;
        }
        ByteReadIntoPlan::AppendBehindHead { source } => {
            // Reader release deliberately retains the first descriptor so a
            // request already exposed to the underlying source stays usable.
            // A subsequent read is queued behind that descriptor.
            let live_head_matches = controller_snapshot
                .live_head(scope, stream, source)
                .is_some();
            let appended =
                live_head_matches && append_descriptor(scope, stream, descriptor.storage);
            if !appended {
                let error = byte_stream_type_error(
                    scope,
                    "Could not append the readable byte stream pull-into descriptor",
                );
                error_stream(scope, stream, error);
                error_read_request(scope, request, error);
            } else {
                enqueue_pending_read(scope, stream, request);
            }
            return appended;
        }
        ByteReadIntoPlan::FillFromQueue | ByteReadIntoPlan::RejectStoredError => {}
    }
    match fill_descriptor_from_queue(scope, stream, &mut descriptor) {
        Ok(true) => {
            if let Err(error) = resolve_descriptor(scope, descriptor, request, false) {
                error_stream(scope, stream, error);
                return false;
            }
            finish_readable_stream_close_if_requested_and_queue_empty(scope, stream);
            return true;
        }
        Ok(false) => {
            if !append_descriptor(scope, stream, descriptor.storage) {
                let error = byte_stream_type_error(
                    scope,
                    "Could not append the readable byte stream pull-into descriptor",
                );
                error_stream(scope, stream, error);
                error_read_request(scope, request, error);
            } else {
                enqueue_pending_read(scope, stream, request);
                finish_readable_stream_close_if_requested_and_queue_empty(scope, stream);
                return true;
            }
        }
        Err(error) => {
            error_stream(scope, stream, error);
            error_read_request(scope, request, error);
        }
    }
    false
}

pub(in crate::context_bootstrap) fn enqueue_auto_allocate_pull_into<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    pending: v8::Local<'s, v8::Object>,
) -> Result<bool, v8::Local<'s, v8::Value>> {
    let Some(plan) = readable_byte_controller_snapshot(scope, stream).plan_auto_allocate() else {
        return Ok(false);
    };
    let descriptor_state = plan.descriptor();
    let size = descriptor_state.byte_length();
    let buffer = v8::ArrayBuffer::new(scope, size);
    let transferred = TransferredView {
        buffer,
        buffer_byte_length: size,
        byte_offset: 0,
        byte_length: size,
        kind: ArrayBufferViewKind::Uint8,
    };
    let descriptor = new_descriptor(
        scope,
        transferred,
        descriptor_state.minimum_fill(),
        descriptor_state.reader_type(),
    )
    .filter(|descriptor| descriptor.state() == Some(descriptor_state))
    .ok_or_else(|| byte_stream_type_error(scope, "Could not allocate a byte stream pull buffer"))?;
    if !append_descriptor(scope, stream, descriptor.storage) {
        return Err(byte_stream_type_error(
            scope,
            "Could not append the byte stream pull buffer",
        ));
    }
    enqueue_pending_read(scope, stream, pending);
    Ok(true)
}

fn enqueue_transferred_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    chunk: &TransferredView<'s>,
) -> Result<(), EnqueueChunkError<'s>> {
    let view = v8::Uint8Array::new(scope, chunk.buffer, chunk.byte_offset, chunk.byte_length)
        .ok_or(EnqueueChunkError::ClosedOrErrored)?;
    enqueue_readable_stream_queue_value(scope, stream, view.into(), chunk.byte_length as f64)
        .map_err(|_| EnqueueChunkError::ClosedOrErrored)
}

pub(crate) fn enqueue_byte_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), EnqueueChunkError<'s>> {
    let enqueue_start = readable_byte_controller_snapshot(scope, stream).plan_enqueue_start();
    if enqueue_start == ByteEnqueueStartPlan::Reject {
        return Err(EnqueueChunkError::ClosedOrErrored);
    }
    let chunk = transfer_view(scope, value).map_err(EnqueueChunkError::Strategy)?;
    if chunk.byte_length == 0 {
        return Err(EnqueueChunkError::Strategy(byte_stream_type_error(
            scope,
            "ReadableByteStreamController.enqueue requires a non-empty view",
        )));
    }
    if let ByteEnqueueStartPlan::Pipe(pipe) = enqueue_start {
        // The existing pipe owner represents its internal default-reader read
        // with this slot instead of a public pending-read promise. Preserve
        // that ownership for byte streams as well: the arriving transferred
        // chunk is queued for the pipe and schedules its drain immediately.
        let view = v8::Uint8Array::new(scope, chunk.buffer, chunk.byte_offset, chunk.byte_length)
            .ok_or(EnqueueChunkError::ClosedOrErrored)?;
        let size = pipe.queue_size(chunk.byte_length) as f64;
        let queue_was_empty = readable_stream_queue_is_empty(scope, stream);
        enqueue_readable_stream_queue_value(scope, stream, view.into(), size)
            .map_err(|_| EnqueueChunkError::ClosedOrErrored)?;
        super::pipe::schedule_pipe_drain_after_incoming_chunk(
            scope,
            stream,
            pipe.fulfills_pending_read(),
            queue_was_empty,
        );
        return Ok(());
    }
    flush_released_descriptor(scope, stream).map_err(EnqueueChunkError::Strategy)?;
    let controller_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
    match controller_snapshot
        .controller()
        .plan_enqueue_after_released()
    {
        ByteEnqueueContinuationPlan::FulfillDefaultDescriptor { source } => {
            let Some(mut descriptor) = controller_snapshot.live_head(scope, stream, source) else {
                return Err(EnqueueChunkError::ClosedOrErrored);
            };
            transfer_descriptor_buffer(scope, &mut descriptor)
                .map_err(EnqueueChunkError::Strategy)?;
            invalidate_byob_request(scope, stream);
            if controller_snapshot
                .live_head(scope, stream, source)
                .is_none()
                || !shift_head_descriptor(scope, stream)
            {
                return Err(EnqueueChunkError::ClosedOrErrored);
            }
            let Some(pending) = super::readable_state::dequeue_first_pending_read(scope, stream)
            else {
                return Err(EnqueueChunkError::ClosedOrErrored);
            };
            let value =
                v8::Uint8Array::new(scope, chunk.buffer, chunk.byte_offset, chunk.byte_length)
                    .ok_or(EnqueueChunkError::ClosedOrErrored)?;
            fulfill_read_request(scope, pending, value.into(), false);
            return Ok(());
        }
        ByteEnqueueContinuationPlan::QueueWithByobDescriptor { source } => {
            let Some(mut descriptor) = controller_snapshot.live_head(scope, stream, source) else {
                return Err(EnqueueChunkError::ClosedOrErrored);
            };
            transfer_descriptor_buffer(scope, &mut descriptor)
                .map_err(EnqueueChunkError::Strategy)?;
            if controller_snapshot
                .live_head(scope, stream, source)
                .is_none()
            {
                return Err(EnqueueChunkError::ClosedOrErrored);
            }
            invalidate_byob_request(scope, stream);
        }
        ByteEnqueueContinuationPlan::Queue => {}
    }
    enqueue_transferred_chunk(scope, stream, &chunk)?;
    process_pending_descriptors(scope, stream).map_err(EnqueueChunkError::Strategy)
}

fn process_pending_descriptors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    let mut descriptor_resolutions = Vec::new();
    collect_pending_descriptor_resolutions(scope, stream, &mut descriptor_resolutions)?;
    let mut default_resolutions = Vec::new();
    collect_pending_default_read_resolutions(scope, stream, &mut default_resolutions)?;
    finish_readable_stream_close_if_requested_and_queue_empty(scope, stream);
    resolve_descriptor_batch(scope, descriptor_resolutions)?;
    for resolution in default_resolutions {
        fulfill_read_request(scope, resolution.pending, resolution.value, false);
    }
    Ok(())
}

fn collect_pending_descriptor_resolutions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    resolutions: &mut Vec<DescriptorResolution<'s>>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    loop {
        flush_released_descriptor(scope, stream)?;
        let Some(mut descriptor) = head_descriptor(scope, stream) else {
            return Ok(());
        };
        if !fill_descriptor_from_queue(scope, stream, &mut descriptor)? {
            return Ok(());
        }
        resolutions.push(take_head_descriptor_resolution(scope, stream, false)?);
    }
}

fn take_head_descriptor_resolution<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    done: bool,
) -> Result<DescriptorResolution<'s>, v8::Local<'s, v8::Value>> {
    let controller_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
    let plan = controller_snapshot
        .controller()
        .plan_take_head_resolution(done)
        .map_err(|error| match error {
            HeadResolutionError::MissingDescriptor => {
                byte_stream_type_error(scope, "The BYOB descriptor is missing")
            }
            HeadResolutionError::ReleasedDescriptor => byte_stream_type_error(
                scope,
                "A released BYOB descriptor has no matching read request",
            ),
            HeadResolutionError::UnalignedDescriptor => byte_stream_type_error(
                scope,
                "The BYOB response is not aligned to the consumer view element size",
            ),
        })?;
    let descriptor = controller_snapshot
        .live_head(scope, stream, plan.source())
        .ok_or_else(|| byte_stream_type_error(scope, "The BYOB descriptor is missing"))?;
    invalidate_byob_request(scope, stream);
    if controller_snapshot
        .live_head(scope, stream, plan.source())
        .is_none()
        || !shift_head_descriptor(scope, stream)
    {
        return Err(byte_stream_type_error(
            scope,
            "Could not remove the BYOB descriptor",
        ));
    }
    let pending = super::readable_state::dequeue_first_pending_read(scope, stream)
        .ok_or_else(|| byte_stream_type_error(scope, "The BYOB read request is missing"))?;
    Ok(DescriptorResolution {
        descriptor,
        pending,
        done,
    })
}

fn collect_pending_default_read_resolutions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    resolutions: &mut Vec<DefaultReadResolution<'s>>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    loop {
        if readable_byte_controller_snapshot(scope, stream).plan_default_read_step()
            == DefaultReadStepPlan::Wait
        {
            return Ok(());
        }
        let value = dequeue_readable_stream_queue_value(scope, stream)
            .map_err(|_| readable_stream_queue_error_value(scope))?
            .ok_or_else(|| readable_stream_queue_error_value(scope))?;
        let pending =
            super::readable_state::dequeue_first_pending_read(scope, stream).ok_or_else(|| {
                byte_stream_type_error(scope, "A queued byte chunk has no matching default read")
            })?;
        resolutions.push(DefaultReadResolution { pending, value });
    }
}

fn flush_released_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    loop {
        let controller_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
        let (source, filled) = match controller_snapshot.controller().plan_flush_released_head() {
            FlushReleasedHeadPlan::Done => return Ok(()),
            FlushReleasedHeadPlan::Flush { source, bytes } => (source, bytes),
        };
        let descriptor = controller_snapshot
            .live_head(scope, stream, source)
            .ok_or_else(|| {
                byte_stream_type_error(scope, "A released BYOB descriptor is invalid")
            })?;
        if !filled.is_empty() {
            let backing = descriptor.buffer.get_backing_store();
            if filled.end() > backing.byte_length() {
                return Err(byte_stream_type_error(
                    scope,
                    "A released BYOB descriptor has invalid bounds",
                ));
            }
            let bytes = backing[filled.offset()..filled.end()]
                .iter()
                .map(std::cell::Cell::get)
                .collect();
            let size = filled.length() as f64;
            let chunk = crate::context_bootstrap::shared::new_uint8_array_from_bytes(scope, bytes)
                .ok_or_else(|| {
                    byte_stream_type_error(scope, "Could not preserve released BYOB bytes")
                })?;
            prepend_readable_stream_queue_value(scope, stream, chunk.into(), size)
                .map_err(|_| readable_stream_queue_error_value(scope))?;
        }
        invalidate_byob_request(scope, stream);
        if controller_snapshot
            .live_head(scope, stream, source)
            .is_none()
            || !shift_head_descriptor(scope, stream)
        {
            return Err(byte_stream_type_error(
                scope,
                "Could not release a BYOB descriptor",
            ));
        }
    }
}

fn invalidate_byob_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    if let Some(request) = stream_slot_object(scope, stream, READABLE_BYTE_STREAM_BYOB_REQUEST_SLOT)
    {
        set_stream_slot_value(
            scope,
            request,
            READABLE_STREAM_BYOB_REQUEST_CONTROLLER_SLOT,
            v8::undefined(scope).into(),
        );
        set_stream_slot_value(
            scope,
            request,
            READABLE_STREAM_BYOB_REQUEST_VIEW_SLOT,
            v8::null(scope).into(),
        );
    }
    set_stream_slot_value(
        scope,
        stream,
        READABLE_BYTE_STREAM_BYOB_REQUEST_SLOT,
        v8::null(scope).into(),
    );
}

fn byob_request_is_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_object(scope, stream, READABLE_BYTE_STREAM_BYOB_REQUEST_SLOT)
        .is_some_and(|current| current.strict_equals(request.into()))
}

pub(in crate::context_bootstrap) fn readable_byte_stream_byob_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    controller: v8::Local<'s, v8::Object>,
    stream: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    let controller_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
    let plan = controller_snapshot.controller().plan_byob_request();
    let creation = match plan {
        ByobRequestPlan::Unavailable => return v8::null(scope).into(),
        ByobRequestPlan::UseCached => {
            return controller_snapshot
                .live_cached_byob_request(scope, stream)
                .map_or_else(|| v8::null(scope).into(), Into::into);
        }
        ByobRequestPlan::Create(creation) => creation,
    };
    let Some(descriptor) = controller_snapshot.live_head(scope, stream, creation.source()) else {
        return v8::null(scope).into();
    };
    let request_range = creation.view();
    let Some(view) = v8::Uint8Array::new(
        scope,
        descriptor.buffer,
        request_range.offset(),
        request_range.length(),
    ) else {
        return v8::null(scope).into();
    };
    let Ok(request) = ReadableStreamByobRequestObjectDeclaration::new(controller, view).bind(scope)
    else {
        return v8::null(scope).into();
    };
    if controller_snapshot
        .live_head(scope, stream, creation.source())
        .is_none()
    {
        return v8::null(scope).into();
    }
    set_stream_slot_value(
        scope,
        stream,
        READABLE_BYTE_STREAM_BYOB_REQUEST_SLOT,
        request.into(),
    );
    request.into()
}

pub(in crate::context_bootstrap) fn readable_byte_stream_pending_byob_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    if matches!(
        readable_byte_controller_snapshot(scope, stream).plan_pending_byob_view(),
        ByobRequestPlan::Unavailable
    ) {
        return None;
    }
    let controller = stream_slot_object(scope, stream, READABLE_STREAM_CONTROLLER_SLOT)?;
    let request = readable_byte_stream_byob_request(scope, controller, stream);
    let request = v8::Local::<v8::Object>::try_from(request).ok()?;
    stream_slot_value(scope, request, READABLE_STREAM_BYOB_REQUEST_VIEW_SLOT)
}

fn validate_respond<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    bytes_written: usize,
) -> Result<(ReadableByteControllerAdapterSnapshot<'s>, RespondPlan), v8::Local<'s, v8::Value>> {
    let Some(descriptor) = head_descriptor(scope, stream) else {
        return Err(byte_stream_type_error(
            scope,
            "The BYOB request has been invalidated",
        ));
    };
    let state = descriptor
        .state()
        .ok_or_else(|| byte_stream_type_error(scope, "The BYOB descriptor is invalid"))?;
    let controller_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
    let context = controller_snapshot
        .controller()
        .respond_context()
        .filter(|context| context.source() == state)
        .ok_or_else(|| {
            byte_stream_type_error(
                scope,
                "The BYOB descriptor changed before the response could be validated",
            )
        })?;
    let plan = context
        .plan_respond(bytes_written)
        .map_err(|error| match error {
            RespondError::ExceedsRemaining => byte_stream_range_error(
                scope,
                "The number of bytes written exceeds the BYOB request view",
            ),
            RespondError::ZeroWhileReadable => byte_stream_type_error(
                scope,
                "ReadableStreamBYOBRequest.respond requires a positive byte count while readable",
            ),
            RespondError::NonZeroWhileClosed => byte_stream_type_error(
                scope,
                "ReadableStreamBYOBRequest.respond only accepts zero after close",
            ),
        })?;
    if controller_snapshot
        .live_head(scope, stream, state)
        .is_none()
    {
        return Err(byte_stream_type_error(
            scope,
            "The BYOB descriptor changed before the response could be validated",
        ));
    }
    Ok((controller_snapshot, plan))
}

fn respond_after_buffer_transfer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    controller_snapshot: ReadableByteControllerAdapterSnapshot<'s>,
    plan: RespondPlan,
) -> Result<(), v8::Local<'s, v8::Value>> {
    let mut descriptor = controller_snapshot
        .live_head(scope, stream, plan.source())
        .ok_or_else(|| {
            byte_stream_type_error(
                scope,
                "The BYOB descriptor changed before the response could be committed",
            )
        })?;
    invalidate_byob_request(scope, stream);
    if let RespondAction::Commit { remainder } = plan.action()
        && !remainder.is_empty()
    {
        let backing = descriptor.buffer.get_backing_store();
        if remainder.end() > backing.byte_length() {
            return Err(byte_stream_type_error(
                scope,
                "The BYOB response remainder has invalid bounds",
            ));
        }
        let bytes = backing[remainder.offset()..remainder.end()]
            .iter()
            .map(std::cell::Cell::get)
            .collect();
        let chunk = crate::context_bootstrap::shared::new_uint8_array_from_bytes(scope, bytes)
            .ok_or_else(|| {
                byte_stream_type_error(scope, "Could not preserve the BYOB remainder")
            })?;
        prepend_readable_stream_queue_value(scope, stream, chunk.into(), remainder.length() as f64)
            .map_err(|_| readable_stream_queue_error_value(scope))?;
    }
    descriptor = controller_snapshot
        .live_head(scope, stream, plan.source())
        .ok_or_else(|| {
            byte_stream_type_error(
                scope,
                "The BYOB descriptor changed before the response could be committed",
            )
        })?;
    descriptor.bytes_filled = plan.descriptor().bytes_filled();
    if !set_descriptor_number(
        scope,
        descriptor.storage,
        DESCRIPTOR_BYTES_FILLED_INDEX,
        descriptor.bytes_filled,
    ) {
        return Err(byte_stream_type_error(
            scope,
            "The BYOB descriptor is invalid",
        ));
    }
    match plan.action() {
        RespondAction::Closed => respond_in_closed_state(scope, stream),
        RespondAction::FlushReleased => {
            // The released reader no longer owns a promise, but its already
            // exposed request still owns the descriptor. Preserve any supplied
            // bytes in the byte queue, remove only that descriptor, then satisfy
            // requests belonging to the replacement reader in FIFO order.
            flush_released_descriptor(scope, stream)?;
            process_pending_descriptors(scope, stream)?;
            maybe_pull_stream(scope, stream);
            Ok(())
        }
        RespondAction::AwaitMore => {
            maybe_pull_stream(scope, stream);
            Ok(())
        }
        RespondAction::Commit { .. } => {
            let mut resolutions = vec![take_head_descriptor_resolution(scope, stream, false)?];
            collect_pending_descriptor_resolutions(scope, stream, &mut resolutions)?;
            finish_readable_stream_close_if_requested_and_queue_empty(scope, stream);
            resolve_descriptor_batch(scope, resolutions)?;
            maybe_pull_stream(scope, stream);
            Ok(())
        }
    }
}

fn respond_in_closed_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    let controller_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
    if let ClosedReleasedHeadPlan::Remove { source } =
        controller_snapshot.controller().plan_closed_released_head()
    {
        let live_matches = controller_snapshot
            .live_head(scope, stream, source)
            .is_some();
        if !live_matches || !shift_head_descriptor(scope, stream) {
            return Err(byte_stream_type_error(
                scope,
                "Could not remove the released BYOB descriptor",
            ));
        }
    }

    // Closed auto-allocation descriptors owned by a default reader are
    // intentionally retained. The default read was already closed with an
    // undefined value, while each `respond(0)` only transfers the descriptor
    // buffer and invalidates the current request. This is the observable
    // Streams Standard behavior (and permits a fresh request to be projected).
    let pending_reads = stream_slot_array(scope, stream, READABLE_STREAM_PENDING_READS_SLOT)
        .map_or(0, |pending| pending.length());
    let mut resolutions = Vec::with_capacity(pending_reads as usize);
    while resolutions.len() < pending_reads as usize {
        let remaining = pending_reads as usize - resolutions.len();
        match readable_byte_controller_snapshot(scope, stream).plan_closed_byob_step(remaining) {
            Ok(ClosedByobStepPlan::Done) => break,
            Ok(ClosedByobStepPlan::Resolve(_)) => {
                resolutions.push(take_head_descriptor_resolution(scope, stream, true)?);
            }
            Err(_) => {
                return Err(byte_stream_type_error(
                    scope,
                    "The BYOB response is not aligned to the consumer view element size",
                ));
            }
        }
    }
    resolve_descriptor_batch(scope, resolutions)
}

pub(in crate::context_bootstrap) fn readable_stream_byob_request_view_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(view) = stream_slot_value(scope, args.this(), READABLE_STREAM_BYOB_REQUEST_VIEW_SLOT)
    else {
        throw_type_error(
            scope,
            "ReadableStreamBYOBRequest.view called on incompatible receiver",
        );
        return;
    };
    rv.set(view);
}

pub(in crate::context_bootstrap) fn readable_stream_byob_request_respond_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(controller) = stream_slot_object(
        scope,
        args.this(),
        READABLE_STREAM_BYOB_REQUEST_CONTROLLER_SLOT,
    ) else {
        throw_type_error(scope, "This ReadableStreamBYOBRequest has been invalidated");
        return;
    };
    let Some(parsed) = webidl::parse_args::<ReadableStreamByobRequestRespondArgs>(scope, &args)
    else {
        return;
    };
    let Some(stream) = stream_slot_object(scope, controller, STREAM_CONTROLLER_STREAM_SLOT) else {
        throw_type_error(scope, "This ReadableStreamBYOBRequest has been invalidated");
        return;
    };
    if !byob_request_is_current(scope, stream, args.this()) {
        throw_type_error(scope, "This ReadableStreamBYOBRequest has been invalidated");
        return;
    }
    let Ok(bytes_written) = usize::try_from(parsed.bytes_written) else {
        throw_range_error(scope, "The number of bytes written is too large");
        return;
    };
    let (controller_snapshot, plan) = match validate_respond(scope, stream, bytes_written) {
        Ok(value) => value,
        Err(error) => {
            scope.throw_exception(error);
            return;
        }
    };
    let Some(mut descriptor) = controller_snapshot.live_head(scope, stream, plan.source()) else {
        throw_type_error(scope, "The BYOB request has been invalidated");
        return;
    };
    if let Err(error) = transfer_descriptor_buffer(scope, &mut descriptor)
        .and_then(|()| respond_after_buffer_transfer(scope, stream, controller_snapshot, plan))
    {
        scope.throw_exception(error);
        return;
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn readable_stream_byob_request_respond_with_new_view_callback<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(controller) = stream_slot_object(
        scope,
        args.this(),
        READABLE_STREAM_BYOB_REQUEST_CONTROLLER_SLOT,
    ) else {
        throw_type_error(scope, "This ReadableStreamBYOBRequest has been invalidated");
        return;
    };
    let Some(stream) = stream_slot_object(scope, controller, STREAM_CONTROLLER_STREAM_SLOT) else {
        throw_type_error(scope, "This ReadableStreamBYOBRequest has been invalidated");
        return;
    };
    if !byob_request_is_current(scope, stream, args.this()) {
        throw_type_error(scope, "This ReadableStreamBYOBRequest has been invalidated");
        return;
    }
    if let Err(error) = respond_byte_stream_with_new_view(scope, stream, args.get(0)) {
        scope.throw_exception(error);
        return;
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn respond_byte_stream_with_new_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) else {
        return Err(byte_stream_type_error(
            scope,
            "respondWithNewView requires an ArrayBufferView",
        ));
    };
    let Some(descriptor) = head_descriptor(scope, stream) else {
        return Err(byte_stream_type_error(
            scope,
            "This ReadableStreamBYOBRequest has been invalidated",
        ));
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(byte_stream_type_error(
            scope,
            "respondWithNewView requires an ArrayBuffer-backed view",
        ));
    };
    let state = descriptor
        .state()
        .ok_or_else(|| byte_stream_type_error(scope, "The BYOB descriptor is invalid"))?;
    let controller_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
    let context = controller_snapshot
        .controller()
        .respond_context()
        .filter(|context| context.source() == state)
        .ok_or_else(|| {
            byte_stream_type_error(
                scope,
                "The BYOB descriptor changed before the response could be validated",
            )
        })?;
    let plan = context
        .plan_respond_with_new_view(ReplacementView {
            buffer_byte_length: buffer.byte_length(),
            byte_offset: view.byte_offset(),
            byte_length: view.byte_length(),
            detached: buffer.was_detached(),
        })
        .map_err(|error| match error {
            ReplacementViewError::Detached => byte_stream_type_error(
                scope,
                "The replacement BYOB view backing buffer is detached",
            ),
            ReplacementViewError::EmptyWhileReadable => byte_stream_type_error(
                scope,
                "A readable byte stream requires a non-empty replacement view",
            ),
            ReplacementViewError::BufferLengthMismatch => byte_stream_range_error(
                scope,
                "The replacement BYOB view buffer has the wrong byte length",
            ),
            ReplacementViewError::InvalidBounds => {
                byte_stream_range_error(scope, "The replacement BYOB view has invalid bounds")
            }
            ReplacementViewError::NonEmptyWhileClosed => byte_stream_type_error(
                scope,
                "A closed byte stream only accepts an empty replacement view",
            ),
        })?;
    let transferred = transfer_view(scope, value)?;
    let descriptor = controller_snapshot
        .live_head(scope, stream, plan.source())
        .ok_or_else(|| {
            byte_stream_type_error(
                scope,
                "The BYOB descriptor changed before the response could be committed",
            )
        })?;
    if !set_descriptor_buffer(scope, descriptor.storage, transferred.buffer) {
        return Err(byte_stream_type_error(
            scope,
            "Could not replace the BYOB descriptor buffer",
        ));
    }
    respond_after_buffer_transfer(scope, stream, controller_snapshot, plan)
}

pub(in crate::context_bootstrap) fn release_byte_stream_reader<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let controller_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
    let (source, next_state) = match controller_snapshot.controller().plan_release_reader() {
        ReleaseReaderPlan::None => return,
        ReleaseReaderPlan::RetainReleasedHead { source, descriptor } => (source, descriptor),
    };
    let Some(descriptor) = controller_snapshot.live_head(scope, stream, source) else {
        return;
    };
    let next = internal_array(scope);
    let _ = define_v8_array_data_property(scope, next, 0, descriptor.storage.into());
    if controller_snapshot
        .live_head(scope, stream, source)
        .is_none()
    {
        return;
    }
    let _ = set_descriptor_number(
        scope,
        descriptor.storage,
        DESCRIPTOR_READER_TYPE_INDEX,
        next_state.reader_type() as usize,
    );
    set_stream_slot_value(
        scope,
        stream,
        READABLE_BYTE_STREAM_PENDING_PULL_INTOS_SLOT,
        next.into(),
    );
}

pub(in crate::context_bootstrap) fn reset_byte_stream_pending_pull_intos<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    clear_descriptors(scope, stream);
}

pub(in crate::context_bootstrap) fn prepare_readable_byte_stream_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    readable_byte_controller_snapshot(scope, stream)
        .validate_close()
        .map_err(|_| {
            byte_stream_type_error(
                scope,
                "Cannot close a readable byte stream with a partially filled element",
            )
        })
}

/// Returns true when close must leave BYOB read-into requests pending until
/// the underlying source acknowledges the terminal request with `respond(0)`.
pub(in crate::context_bootstrap) fn finish_readable_byte_stream_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    // A released descriptor can precede requests from the replacement reader.
    // Closing must leave active BYOB reads pending until the source acknowledges
    // the terminal request with `respond(0)`. Default-reader reads still close
    // immediately, while their auto-allocation descriptor remains observable.
    readable_byte_controller_snapshot(scope, stream).plan_finish_close()
        == FinishByteClosePlan::WaitForByobResponse
}

pub(in crate::context_bootstrap) fn finish_byte_stream_tee_branch_close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Result<(), v8::Local<'s, v8::Value>> {
    if !readable_stream_is_byte_stream(scope, stream) || !readable_stream_closed(scope, stream) {
        return Ok(());
    }
    invalidate_byob_request(scope, stream);
    respond_in_closed_state(scope, stream)
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::*;

    fn test_descriptor<'s>(scope: &mut v8::PinScope<'s, '_>) -> PullIntoDescriptor<'s> {
        let size = 8;
        new_descriptor(
            scope,
            TransferredView {
                buffer: v8::ArrayBuffer::new(scope, size),
                buffer_byte_length: size,
                byte_offset: 0,
                byte_length: size,
                kind: ArrayBufferViewKind::Uint8,
            },
            1,
            PullIntoReaderType::Byob,
        )
        .expect("valid test descriptor")
    }

    #[test]
    fn readable_byte_stream_adapter_snapshot_rejects_same_metadata_replacements() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let stream = v8::Object::new(scope);
        initialize_readable_byte_stream_state(scope, stream, None);
        let first = test_descriptor(scope);
        assert!(append_descriptor(scope, stream, first.storage));
        let source = first.state().expect("valid descriptor state");
        let snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
        assert!(snapshot.live_head(scope, stream, source).is_some());

        let replacement_list = internal_array(scope);
        assert!(
            define_v8_array_data_property(scope, replacement_list, 0, first.storage.into())
                .is_some()
        );
        set_stream_slot_value(
            scope,
            stream,
            READABLE_BYTE_STREAM_PENDING_PULL_INTOS_SLOT,
            replacement_list.into(),
        );
        assert!(snapshot.live_head(scope, stream, source).is_none());

        let second_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
        let second = test_descriptor(scope);
        assert_eq!(second.state(), Some(source));
        assert!(
            define_v8_array_data_property(scope, replacement_list, 0, second.storage.into())
                .is_some()
        );
        assert!(second_snapshot.live_head(scope, stream, source).is_none());

        let first_request = v8::Object::new(scope);
        set_stream_slot_value(
            scope,
            stream,
            READABLE_BYTE_STREAM_BYOB_REQUEST_SLOT,
            first_request.into(),
        );
        let request_snapshot = readable_byte_controller_adapter_snapshot(scope, stream);
        assert!(
            request_snapshot
                .live_cached_byob_request(scope, stream)
                .is_some()
        );
        let replacement_request = v8::Object::new(scope);
        set_stream_slot_value(
            scope,
            stream,
            READABLE_BYTE_STREAM_BYOB_REQUEST_SLOT,
            replacement_request.into(),
        );
        assert!(
            request_snapshot
                .live_cached_byob_request(scope, stream)
                .is_none()
        );
    }
}
