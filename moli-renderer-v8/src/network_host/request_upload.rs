//! Author streams are read on their script owner, one snapshotted chunk at a
//! time. Only bytes and readiness notifications cross to the resource runner.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    sync::Arc,
};

use moli_fetch::{FetchCancelHandle, UploadBody, UploadBodyError, UploadBodySender};

use crate::{
    context_bootstrap::{
        begin_readable_stream_body_consumption, cancel_readable_stream_for_fetch,
        maybe_pull_stream, prepare_readable_stream_read_with_steps,
    },
    network::RendererResourceTaskRunner,
    util::{get_private_value, set_null_prototype, set_private_value, v8str},
};

const NATIVE: &str = "__moliRequestUploadNative";
const STREAM: &str = "__moliRequestUploadStream";
const RESOLVER: &str = "__moliRequestUploadResolver";
const STEPS: &str = "__moliRequestUploadSteps";

#[derive(Clone)]
pub(crate) struct RequestUpload(Rc<Upload>);

struct Upload {
    consumer: v8::Global<v8::Object>,
    sender: RefCell<Option<UploadBodySender>>,
    stopped: Cell<bool>,
    reading: Cell<bool>,
    runner: RendererResourceTaskRunner,
    ready: Arc<dyn Fn(Result<(), UploadBodyError>) + Send + Sync>,
    cancel: FetchCancelHandle,
}

impl RequestUpload {
    pub(crate) fn new<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        stream: v8::Local<'s, v8::Object>,
        resolver: v8::Local<'s, v8::PromiseResolver>,
        already_locked: bool,
        runner: RendererResourceTaskRunner,
        cancel: FetchCancelHandle,
        ready: impl Fn(Result<(), UploadBodyError>) + Send + Sync + 'static,
    ) -> Result<(Self, UploadBody), v8::Local<'s, v8::Value>> {
        if !already_locked && !begin_readable_stream_body_consumption(scope, stream) {
            return Err(v8::Exception::type_error(
                scope,
                v8str(scope, "Body stream is locked"),
            ));
        }
        let consumer = v8::Object::new(scope);
        set_null_prototype(scope, consumer);
        set_private_value(scope, consumer, STREAM, stream.into());
        set_private_value(scope, consumer, RESOLVER, resolver.into());
        let steps = [
            v8::Function::builder(chunk)
                .data(consumer.into())
                .build(scope),
            v8::Function::builder(close)
                .data(consumer.into())
                .build(scope),
            v8::Function::builder(error)
                .data(consumer.into())
                .build(scope),
        ]
        .map(|step| step.expect("request upload read step").into());
        let steps = v8::Array::new_with_elements(scope, &steps);
        set_null_prototype(scope, steps.into());
        set_private_value(scope, consumer, STEPS, steps.into());
        let (sender, body) = UploadBody::channel();
        let upload = Self(Rc::new(Upload {
            consumer: v8::Global::new(scope, consumer),
            sender: RefCell::new(Some(sender)),
            stopped: Cell::new(false),
            reading: Cell::new(false),
            runner,
            ready: Arc::new(ready),
            cancel,
        }));
        // V8 owns only a weak edge to the request. Removing its pending native
        // continuation immediately drops the producer, including at retirement.
        let mut weak = Box::new(Rc::downgrade(&upload.0));
        let external = v8::External::new(scope, (&mut *weak as *mut Weak<Upload>).cast());
        set_private_value(scope, consumer, NATIVE, external.into());
        crate::v8_finalizer::track_context_owned_v8_finalizer(scope, consumer, move || drop(weak));
        Ok((upload, body))
    }

    /// Lock synchronously, but start reads after fetch has installed its abort
    /// listener and released its mutable host/Worker borrow.
    pub(crate) fn start(&self, scope: &mut v8::PinScope<'_, '_>) {
        let consumer = v8::Local::new(scope, &self.0.consumer);
        let callback = v8::Function::builder(read)
            .data(consumer.into())
            .build(scope)
            .expect("request upload start");
        scope.enqueue_microtask(callback);
    }

    pub(crate) fn ready(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        result: Result<(), UploadBodyError>,
    ) {
        if result.is_err() {
            self.0.stopped.set(true);
            self.0.sender.borrow_mut().take();
            // The network owner selects the terminal outcome. In particular,
            // retiring the original upload for a 303 must not reject fetch.
            return;
        }
        self.0.reading.set(false);
        let consumer = v8::Local::new(scope, &self.0.consumer);
        read_next(scope, consumer, &self.0);
    }

    pub(crate) fn finish(&self, scope: &mut v8::PinScope<'_, '_>) {
        let reason = v8::undefined(scope).into();
        self.stop(scope, reason);
    }

    pub(crate) fn stop<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        reason: v8::Local<'s, v8::Value>,
    ) {
        self.0.stopped.set(true);
        self.0.sender.borrow_mut().take();
        let consumer = v8::Local::new(scope, &self.0.consumer);
        let stream = v8::Local::<v8::Object>::try_from(
            get_private_value(scope, consumer, STREAM).expect("request upload stream"),
        )
        .expect("request upload stream object");
        cancel_readable_stream_for_fetch(scope, stream, reason);
    }
}

fn native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, Rc<Upload>)> {
    // Teardown can free the context-owned weak allocation while author code
    // still retains the stream and its internal read callbacks.
    let (binding, _) = crate::common_runtime::CommonRuntimeBinding::from_scope(scope)?;
    binding.callback_execution_context(scope)?;
    let consumer = v8::Local::<v8::Object>::try_from(data).ok()?;
    let external =
        v8::Local::<v8::External>::try_from(get_private_value(scope, consumer, NATIVE)?).ok()?;
    // SAFETY: only new() installs this private typed weak allocation. Its
    // guaranteed finalizer lives with this context; it owns no V8 globals.
    let state = unsafe { &*external.value().cast::<Weak<Upload>>() }.upgrade()?;
    Some((consumer, state))
}

fn read_next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    consumer: v8::Local<'s, v8::Object>,
    state: &Upload,
) {
    if state.stopped.get() || state.cancel.is_cancelled() || state.reading.replace(true) {
        return;
    }
    let stream = v8::Local::<v8::Object>::try_from(
        get_private_value(scope, consumer, STREAM).expect("request upload stream"),
    )
    .expect("request upload stream object");
    let steps = v8::Local::<v8::Array>::try_from(
        get_private_value(scope, consumer, STEPS).expect("request upload read steps"),
    )
    .expect("request upload read step array");
    let mut callbacks = Vec::with_capacity(3);
    for index in 0..3 {
        callbacks.push(
            v8::Local::<v8::Function>::try_from(
                steps
                    .get_index(scope, index)
                    .expect("request upload read callback"),
            )
            .expect("request upload read function"),
        );
    }
    let pull = prepare_readable_stream_read_with_steps(
        scope,
        stream,
        callbacks[0],
        callbacks[1],
        callbacks[2],
    );
    if pull && !state.stopped.get() {
        maybe_pull_stream(scope, stream);
    }
}

fn read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some((consumer, state)) = native(scope, args.data()) {
        read_next(scope, consumer, &state);
    }
}

fn fail<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    consumer: v8::Local<'s, v8::Object>,
    state: Rc<Upload>,
    reason: v8::Local<'s, v8::Value>,
) {
    if state.stopped.replace(true) {
        return;
    }
    state.cancel.cancel();
    state.sender.borrow_mut().take();
    let value =
        get_private_value(scope, consumer, RESOLVER).expect("request upload fetch resolver");
    let object = v8::Local::<v8::Object>::try_from(value).expect("request upload resolver object");
    // SAFETY: new() stores the intrinsic PromiseResolver in this private slot.
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) };
    resolver.reject(scope, reason);
    RequestUpload(state).stop(scope, reason);
}

fn chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((consumer, state)) = native(scope, args.data()) else {
        return;
    };
    if state.stopped.get() {
        return;
    }
    let Ok(chunk) = v8::Local::<v8::Uint8Array>::try_from(args.get(0)) else {
        let reason = v8::Exception::type_error(scope, v8str(scope, "Failed to fetch"));
        fail(scope, consumer, state, reason);
        return;
    };
    // Copy the represented view before another pull can mutate or detach it.
    let mut bytes = vec![0; chunk.byte_length()];
    let count = chunk.copy_contents(&mut bytes);
    bytes.truncate(count);
    let Some(send) = state
        .sender
        .borrow()
        .as_ref()
        .map(|sender| sender.send(bytes.into()))
    else {
        return;
    };
    let ready = state.ready.clone();
    state.runner.spawn(async move {
        ready(send.await);
    });
}

fn close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((_, state)) = native(scope, args.data()) else {
        return;
    };
    if state.stopped.replace(true) {
        return;
    }
    let sender = state.sender.borrow_mut().take();
    if let Some(sender) = sender {
        sender.finish();
    }
}

fn error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some((consumer, state)) = native(scope, args.data()) {
        let reason = v8::Exception::type_error(scope, v8str(scope, "Failed to fetch"));
        fail(scope, consumer, state, reason);
    }
}
