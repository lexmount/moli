use super::*;
use moli_v8_util::set_null_prototype;
use moli_webapi_declare::define_array_data_property as define_v8_array_data_property;

const READABLE_STREAM_QUEUE_TOTAL_SIZE_SLOT: &str = "__moliReadableStreamQueueTotalSize";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::streams) enum ReadableStreamQueueError {
    Missing,
    InvalidEntry,
    Replaced,
    CommitFailed,
}

struct ReadableStreamQueueSnapshot<'s> {
    queue: v8::Local<'s, v8::Array>,
    bounds: moli_streams::queue::QueueBounds,
    total_size: moli_streams::queue::QueueTotalSize,
}

// ReadableStream queues live in private V8 slots, but their arrays still belong
// to the page realm. Keep all access in this module so no queue handle survives
// a call into user JavaScript. A replacement array's identity is the queue's
// generation token for two-phase operations.
fn new_readable_stream_queue<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Array> {
    let queue = v8::Array::new(scope, 0);
    set_null_prototype(scope, queue.into());
    set_readable_stream_queue_total_size(
        scope,
        queue,
        moli_streams::queue::QueueTotalSize::default(),
    );
    queue
}

fn readable_stream_queue_stored_total_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    queue: v8::Local<'s, v8::Array>,
) -> moli_streams::queue::QueueTotalSize {
    moli_streams::queue::QueueTotalSize::from_stored(
        stream_slot_number(scope, queue.into(), READABLE_STREAM_QUEUE_TOTAL_SIZE_SLOT)
            .unwrap_or(0.0),
    )
}

fn set_readable_stream_queue_total_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    queue: v8::Local<'s, v8::Array>,
    total_size: moli_streams::queue::QueueTotalSize,
) {
    let total_size = v8::Number::new(scope, total_size.value());
    set_stream_slot_value(
        scope,
        queue.into(),
        READABLE_STREAM_QUEUE_TOTAL_SIZE_SLOT,
        total_size.into(),
    );
}

fn readable_stream_queue_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Result<ReadableStreamQueueSnapshot<'s>, ReadableStreamQueueError> {
    let queue = stream_slot_array(scope, stream, READABLE_STREAM_QUEUE_SLOT)
        .ok_or(ReadableStreamQueueError::Missing)?;
    let head = readable_stream_queue_head(scope, stream);
    let bounds = moli_streams::queue::QueueBounds::new(head as usize, queue.length() as usize)
        .map_err(|_| ReadableStreamQueueError::InvalidEntry)?;
    let total_size = readable_stream_queue_stored_total_size(scope, queue);
    Ok(ReadableStreamQueueSnapshot {
        queue,
        bounds,
        total_size,
    })
}

fn verify_readable_stream_queue_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    snapshot: &ReadableStreamQueueSnapshot<'s>,
) -> Result<(), ReadableStreamQueueError> {
    let current = stream_slot_array(scope, stream, READABLE_STREAM_QUEUE_SLOT)
        .ok_or(ReadableStreamQueueError::Missing)?;
    if !current.strict_equals(snapshot.queue.into())
        || readable_stream_queue_head(scope, stream) as usize != snapshot.bounds.head()
        || readable_stream_queue_stored_total_size(scope, current)
            .value()
            .to_bits()
            != snapshot.total_size.value().to_bits()
    {
        return Err(ReadableStreamQueueError::Replaced);
    }
    Ok(())
}

fn replace_readable_stream_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    queue: v8::Local<'s, v8::Array>,
) {
    set_stream_slot_value(scope, stream, READABLE_STREAM_QUEUE_SLOT, queue.into());
    set_readable_stream_queue_head(scope, stream, 0);
}

fn readable_stream_queue_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    size: f64,
) -> Result<v8::Local<'s, v8::Array>, ReadableStreamQueueError> {
    let entry = v8::Array::new(scope, 2);
    set_null_prototype(scope, entry.into());
    define_v8_array_data_property(scope, entry, 0, value)
        .ok_or(ReadableStreamQueueError::CommitFailed)?;
    let size = v8::Number::new(scope, size);
    define_v8_array_data_property(scope, entry, 1, size.into())
        .ok_or(ReadableStreamQueueError::CommitFailed)?;
    Ok(entry)
}

fn readable_stream_queue_entry_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, ReadableStreamQueueError> {
    let entry = v8::Local::<v8::Array>::try_from(entry)
        .map_err(|_| ReadableStreamQueueError::InvalidEntry)?;
    entry
        .get_index(scope, 0)
        .ok_or(ReadableStreamQueueError::InvalidEntry)
}

fn readable_stream_queue_entry_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Value>,
) -> Result<f64, ReadableStreamQueueError> {
    let entry = v8::Local::<v8::Array>::try_from(entry)
        .map_err(|_| ReadableStreamQueueError::InvalidEntry)?;
    entry
        .get_index(scope, 1)
        .and_then(|value| value.number_value(scope))
        .ok_or(ReadableStreamQueueError::InvalidEntry)
}

fn append_readable_stream_queue_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    queue: v8::Local<'s, v8::Array>,
    entry: v8::Local<'s, v8::Array>,
) -> Result<(), ReadableStreamQueueError> {
    define_v8_array_data_property(scope, queue, queue.length(), entry.into())
        .ok_or(ReadableStreamQueueError::CommitFailed)
}

pub(in crate::streams) fn enqueue_readable_stream_queue_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    size: f64,
) -> Result<(), ReadableStreamQueueError> {
    // Build the entry first, then resolve the queue at the commit point. This
    // function itself invokes no user code, so the queue cannot be replaced
    // between lookup and append.
    let entry = readable_stream_queue_entry(scope, value, size)?;
    let queue = stream_slot_array(scope, stream, READABLE_STREAM_QUEUE_SLOT)
        .ok_or(ReadableStreamQueueError::Missing)?;
    let next_total = readable_stream_queue_stored_total_size(scope, queue)
        .plan_enqueue(size)
        .next();
    append_readable_stream_queue_entry(scope, queue, entry)?;
    set_readable_stream_queue_total_size(scope, queue, next_total);
    Ok(())
}

pub(in crate::streams) fn prepend_readable_stream_queue_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    size: f64,
) -> Result<(), ReadableStreamQueueError> {
    let snapshot = readable_stream_queue_snapshot(scope, stream)?;
    let replacement = new_readable_stream_queue(scope);
    let entry = readable_stream_queue_entry(scope, value, size)?;
    append_readable_stream_queue_entry(scope, replacement, entry)?;
    for index in snapshot.bounds.head() as u32..snapshot.queue.length() {
        let entry = snapshot
            .queue
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            .ok_or(ReadableStreamQueueError::InvalidEntry)?;
        // Validate both fields before reusing the internal entry object in the
        // replacement queue. Neither array is exposed to author code.
        let _ = readable_stream_queue_entry_value(scope, entry.into())?;
        let _ = readable_stream_queue_entry_size(scope, entry.into())?;
        append_readable_stream_queue_entry(scope, replacement, entry)?;
    }
    verify_readable_stream_queue_snapshot(scope, stream, &snapshot)?;
    let next_total = snapshot.total_size.plan_enqueue(size).next();
    set_readable_stream_queue_total_size(scope, replacement, next_total);
    replace_readable_stream_queue(scope, stream, replacement);
    Ok(())
}

pub(in crate::streams) fn readable_stream_queue_exists<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    stream_slot_array(scope, stream, READABLE_STREAM_QUEUE_SLOT).is_some()
}

pub(in crate::streams) fn readable_stream_queue_head<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> u32 {
    stream_slot_number(scope, stream, READABLE_STREAM_QUEUE_HEAD_SLOT).unwrap_or(0.0) as u32
}

fn set_readable_stream_queue_head<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    head: u32,
) {
    let head = v8::Number::new(scope, head as f64);
    set_stream_slot_value(scope, stream, READABLE_STREAM_QUEUE_HEAD_SLOT, head.into());
}

pub(in crate::streams) fn readable_stream_queue_is_empty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(queue) = stream_slot_array(scope, stream, READABLE_STREAM_QUEUE_SLOT) else {
        return true;
    };
    match moli_streams::queue::QueueBounds::new(
        readable_stream_queue_head(scope, stream) as usize,
        queue.length() as usize,
    ) {
        Ok(bounds) => bounds.is_empty(),
        Err(_) => true,
    }
}

pub(in crate::streams) fn reset_readable_stream_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) {
    let queue = new_readable_stream_queue(scope);
    replace_readable_stream_queue(scope, stream, queue);
}

pub(in crate::streams) fn dequeue_readable_stream_queue_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> Result<Option<v8::Local<'s, v8::Value>>, ReadableStreamQueueError> {
    let snapshot = readable_stream_queue_snapshot(scope, stream)?;
    let Some(plan) = snapshot.bounds.dequeue() else {
        return Ok(None);
    };
    let entry = snapshot
        .queue
        .get_index(scope, plan.index() as u32)
        .ok_or(ReadableStreamQueueError::InvalidEntry)?;
    let value = readable_stream_queue_entry_value(scope, entry)?;
    let size = readable_stream_queue_entry_size(scope, entry)?;
    let next_total = snapshot.total_size.plan_dequeue(size).next();
    verify_readable_stream_queue_snapshot(scope, stream, &snapshot)?;

    match plan.remainder() {
        moli_streams::queue::QueueRemainderPlan::Reset => {
            let replacement = new_readable_stream_queue(scope);
            set_readable_stream_queue_total_size(scope, replacement, next_total);
            replace_readable_stream_queue(scope, stream, replacement);
        }
        moli_streams::queue::QueueRemainderPlan::AdvanceHead(next_head) => {
            define_v8_array_data_property(
                scope,
                snapshot.queue,
                plan.index() as u32,
                v8::undefined(scope).into(),
            )
            .ok_or(ReadableStreamQueueError::CommitFailed)?;
            set_readable_stream_queue_head(scope, stream, next_head as u32);
            set_readable_stream_queue_total_size(scope, snapshot.queue, next_total);
        }
    }
    Ok(Some(value))
}

pub fn readable_stream_queue_total_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
) -> f64 {
    readable_stream_queue_snapshot(scope, stream)
        .map(|snapshot| snapshot.total_size.value())
        .unwrap_or(0.0)
}

pub(in crate::streams) fn take_byte_stream_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stream: v8::Local<'s, v8::Object>,
    capacity: usize,
) -> Result<Option<Vec<u8>>, ReadableStreamQueueError> {
    let snapshot = readable_stream_queue_snapshot(scope, stream)?;
    if snapshot.bounds.is_empty() {
        return Ok(None);
    }
    if capacity == 0 {
        return Ok(Some(Vec::new()));
    }

    // Prepare only the consumed prefix and a replacement queue without
    // mutating the live generation. Untouched entries retain their buffers and
    // identity, and a partially consumed entry becomes a zero-copy tail view.
    // This keeps a small BYOB read O(read size), not O(total queued body size).
    let mut queued = Vec::with_capacity(capacity);
    let mut remaining = capacity;
    let mut next_total = snapshot.total_size;
    let replacement = new_readable_stream_queue(scope);
    for index in snapshot.bounds.head() as u32..snapshot.queue.length() {
        let entry_value = snapshot
            .queue
            .get_index(scope, index)
            .ok_or(ReadableStreamQueueError::InvalidEntry)?;
        let entry = v8::Local::<v8::Array>::try_from(entry_value)
            .map_err(|_| ReadableStreamQueueError::InvalidEntry)?;
        let value = readable_stream_queue_entry_value(scope, entry.into())?;
        let _ = readable_stream_queue_entry_size(scope, entry.into())?;
        if remaining == 0 {
            append_readable_stream_queue_entry(scope, replacement, entry)?;
            continue;
        }

        let view = v8::Local::<v8::ArrayBufferView>::try_from(value)
            .map_err(|_| ReadableStreamQueueError::InvalidEntry)?;
        let buffer = view
            .buffer(scope)
            .ok_or(ReadableStreamQueueError::InvalidEntry)?;
        if buffer.was_detached() {
            return Err(ReadableStreamQueueError::InvalidEntry);
        }
        let start = view.byte_offset();
        let byte_length = view.byte_length();
        let end = start
            .checked_add(byte_length)
            .ok_or(ReadableStreamQueueError::InvalidEntry)?;
        let backing = buffer.get_backing_store();
        if end > backing.byte_length() {
            return Err(ReadableStreamQueueError::InvalidEntry);
        }
        let consumed = remaining.min(byte_length);
        queued.extend(
            backing[start..start + consumed]
                .iter()
                .map(std::cell::Cell::get),
        );
        remaining -= consumed;
        next_total = next_total.plan_dequeue(consumed as f64).next();

        if consumed < byte_length {
            let tail_length = byte_length - consumed;
            let tail = v8::Uint8Array::new(scope, buffer, start + consumed, tail_length)
                .ok_or(ReadableStreamQueueError::CommitFailed)?;
            let tail_entry = readable_stream_queue_entry(scope, tail.into(), tail_length as f64)?;
            append_readable_stream_queue_entry(scope, replacement, tail_entry)?;
        }
    }

    verify_readable_stream_queue_snapshot(scope, stream, &snapshot)?;
    set_readable_stream_queue_total_size(scope, replacement, next_total);
    replace_readable_stream_queue(scope, stream, replacement);
    Ok(Some(queued))
}

pub(in crate::streams) fn readable_stream_queue_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    v8::Exception::type_error(
        scope,
        v8str(scope, "ReadableStream internal queue state is invalid"),
    )
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::*;
    use moli_v8_util::new_uint8_array_from_bytes;

    #[test]
    fn byte_take_failure_preserves_the_live_queue_generation() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let stream = v8::Object::new(scope);
        let queue = new_readable_stream_queue(scope);
        let bytes =
            new_uint8_array_from_bytes(scope, vec![1, 2, 3]).expect("valid byte queue entry");
        let first =
            readable_stream_queue_entry(scope, bytes.into(), 3.0).expect("valid queue entry");
        append_readable_stream_queue_entry(scope, queue, first).expect("first queue append");
        let invalid_value = v8str(scope, "not bytes");
        let invalid = readable_stream_queue_entry(scope, invalid_value.into(), 1.0)
            .expect("structurally valid queue entry");
        append_readable_stream_queue_entry(scope, queue, invalid).expect("second queue append");
        set_stream_slot_value(scope, stream, READABLE_STREAM_QUEUE_SLOT, queue.into());
        set_readable_stream_queue_head(scope, stream, 0);

        assert!(matches!(
            take_byte_stream_bytes(scope, stream, 4),
            Err(ReadableStreamQueueError::InvalidEntry)
        ));

        let current = stream_slot_array(scope, stream, READABLE_STREAM_QUEUE_SLOT)
            .expect("queue remains installed");
        assert!(current.strict_equals(queue.into()));
        assert_eq!(readable_stream_queue_head(scope, stream), 0);
        assert_eq!(current.length(), 2);
        let first = current.get_index(scope, 0).expect("first entry remains");
        let first = readable_stream_queue_entry_value(scope, first).expect("first entry value");
        assert_eq!(value_buffer_source_bytes(scope, first), Some(vec![1, 2, 3]));
        let second = current.get_index(scope, 1).expect("second entry remains");
        let second = readable_stream_queue_entry_value(scope, second).expect("second entry value");
        assert!(second.is_string());

        // A smaller read need not inspect or copy the untouched invalid value.
        // It commits only the requested prefix and leaves a view over the first
        // entry's backing store ahead of the untouched second entry.
        assert_eq!(
            take_byte_stream_bytes(scope, stream, 2),
            Ok(Some(vec![1, 2]))
        );
        let current = stream_slot_array(scope, stream, READABLE_STREAM_QUEUE_SLOT)
            .expect("replacement queue is installed");
        assert!(!current.strict_equals(queue.into()));
        assert_eq!(current.length(), 2);
        let tail = current.get_index(scope, 0).expect("tail entry remains");
        let tail = readable_stream_queue_entry_value(scope, tail).expect("tail entry value");
        assert_eq!(value_buffer_source_bytes(scope, tail), Some(vec![3]));
        let untouched = current
            .get_index(scope, 1)
            .expect("untouched entry remains");
        let untouched =
            readable_stream_queue_entry_value(scope, untouched).expect("untouched entry value");
        assert!(untouched.is_string());
    }
}
