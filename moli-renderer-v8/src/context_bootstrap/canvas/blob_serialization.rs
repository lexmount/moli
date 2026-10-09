use moli_webidl_callback::WebIdlCallbackFunction;
use tokio::sync::oneshot;

use crate::{
    blob::build_blob_object,
    context_bootstrap::new_dom_exception_value,
    host::report_event_callback_exception,
    native_bridge::JsContextHost,
    util::context_host_ptr_from_global_bridge,
    webidl,
    window_webidl_callback::{
        WindowWebIdlCallbackFunction, WindowWebIdlCallbackFunctionOutcome,
        invoke_window_webidl_callback_function,
    },
};

use super::backing_store::canvas_like_pixels_copy;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLCanvasElement.toBlob")]
struct CanvasToBlobArgs {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
    #[webidl(converter = "dom_string", default = "image/png".to_owned())]
    mime_type: String,
}

pub(crate) enum CanvasBlobTaskEffect {
    Completed,
    Retired,
}

pub(crate) struct CanvasBlobFile {
    bytes: Vec<u8>,
    mime_type: &'static str,
}

/// Immutable serialization input; no DOM or V8 residence crosses threads.
pub(crate) struct CanvasBlobEncodeJob {
    pixels: Option<(Vec<u8>, u32, u32)>,
    jpeg_quality: Option<u8>,
}

impl CanvasBlobEncodeJob {
    pub(super) fn new(
        pixels: Option<(Vec<u8>, u32, u32)>,
        mime_type: &str,
        quality: Option<f64>,
    ) -> Self {
        Self {
            pixels,
            jpeg_quality: mime_type.eq_ignore_ascii_case("image/jpeg").then(|| {
                quality
                    .filter(|quality| (0.0..=1.0).contains(quality))
                    .map(|quality| (quality * 100.0).round() as u8)
                    .unwrap_or(92)
            }),
        }
    }

    pub(crate) fn encode(self) -> Option<CanvasBlobFile> {
        let (mut pixels, width, height) = self.pixels?;
        if width == 0 || height == 0 {
            return None;
        }
        let (bytes, mime_type) = match self.jpeg_quality {
            Some(quality) => {
                // JPEG has no alpha channel. Composite over opaque black before
                // passing the RGBA surface to the existing JPEG encoder.
                moli_canvas::premultiply_rgba8_in_place(&mut pixels)?;
                for pixel in pixels.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
                let image = moli_image::RgbaImage::try_new(width, height, pixels).ok()?;
                (
                    moli_image::encode_jpeg(&image, quality).ok()?.bytes,
                    "image/jpeg",
                )
            }
            None => (
                moli_image::encode_png_rgba8(width, height, &pixels)
                    .ok()?
                    .bytes,
                "image/png",
            ),
        };
        Some(CanvasBlobFile { bytes, mime_type })
    }

    pub(crate) fn spawn(
        self,
        encoded_tx: oneshot::Sender<Option<CanvasBlobFile>>,
        ready: impl FnOnce() + Send + 'static,
    ) {
        let encode = move || {
            if encoded_tx.send(self.encode()).is_ok() {
                ready();
            }
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(encode);
        } else {
            std::thread::spawn(encode);
        }
    }
}

/// The resolver and canvas relevant realm stay on their owning event loop.
pub(crate) struct CanvasBlobPromise {
    resolver: v8::Global<v8::PromiseResolver>,
    canvas_context: v8::Global<v8::Context>,
}

impl CanvasBlobPromise {
    pub(crate) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        resolver: v8::Local<'_, v8::PromiseResolver>,
        canvas_context: v8::Local<'_, v8::Context>,
    ) -> Self {
        Self {
            resolver: v8::Global::new(scope, resolver),
            canvas_context: v8::Global::new(scope, canvas_context),
        }
    }

    pub(crate) fn settle(self, scope: &mut v8::PinScope<'_, '_>, file: Option<CanvasBlobFile>) {
        let context = v8::Local::new(scope, &self.canvas_context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::Local::new(scope, &self.resolver);
        let blob =
            file.and_then(|file| build_blob_object(scope, file.bytes, file.mime_type.to_owned()));
        if let Some(blob) = blob {
            let _ = resolver.resolve(scope, blob.into());
        } else {
            let error =
                new_dom_exception_value(scope, "The canvas could not be encoded.", "EncodingError");
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(crate) enum CanvasBlobCompletion {
    Callback(WindowWebIdlCallbackFunction),
    Promise(CanvasBlobPromise),
}

impl CanvasBlobCompletion {
    pub(crate) fn callback(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        callback: WebIdlCallbackFunction,
    ) -> Self {
        Self::Callback(WindowWebIdlCallbackFunction::new(scope, host, callback))
    }
}

/// Completion residence and the canvas relevant realm never leave the V8 Host.
/// Only the encoder's owned result crosses the blocking-job boundary.
pub(crate) struct CanvasBlobTask {
    completion: CanvasBlobCompletion,
    canvas_context: v8::Global<v8::Context>,
    encoded: oneshot::Receiver<Option<CanvasBlobFile>>,
}

impl CanvasBlobTask {
    pub(crate) fn context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Context> {
        v8::Local::new(scope, &self.canvas_context)
    }

    pub(crate) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        completion: CanvasBlobCompletion,
        canvas_context: v8::Local<'_, v8::Context>,
        encoded: oneshot::Receiver<Option<CanvasBlobFile>>,
    ) -> Self {
        Self {
            completion,
            canvas_context: v8::Global::new(scope, canvas_context),
            encoded,
        }
    }

    pub(crate) fn invoke(
        mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) -> CanvasBlobTaskEffect {
        // Publication sends the result before admitting this scheduler task.
        let encoded = self
            .encoded
            .try_recv()
            .expect("ready canvas blob task must retain its encoder result");
        let context = v8::Local::new(scope, &self.canvas_context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let callback = match self.completion {
            CanvasBlobCompletion::Promise(promise) => {
                promise.settle(scope, encoded);
                return CanvasBlobTaskEffect::Completed;
            }
            CanvasBlobCompletion::Callback(callback) => callback,
        };
        let blob = match encoded {
            Some(file) => build_blob_object(scope, file.bytes, file.mime_type.to_owned())
                .map(Into::into)
                .unwrap_or_else(|| v8::null(scope).into()),
            None => v8::null(scope).into(),
        };
        let callback = callback.prepare(scope);
        let relevant_identity = callback.relevant_identity();
        let receiver = v8::undefined(scope);
        match invoke_window_webidl_callback_function(
            scope,
            host_ptr,
            "BlobCallback",
            "HTMLCanvasElement.toBlob callback threw",
            "HTMLCanvasElement.toBlob callback",
            &callback,
            receiver.into(),
            &[blob],
        ) {
            WindowWebIdlCallbackFunctionOutcome::Returned => CanvasBlobTaskEffect::Completed,
            WindowWebIdlCallbackFunctionOutcome::Threw(report) => {
                report_event_callback_exception(
                    scope,
                    host_ptr,
                    "canvas",
                    relevant_identity,
                    None,
                    &report,
                );
                CanvasBlobTaskEffect::Completed
            }
            WindowWebIdlCallbackFunctionOutcome::Retired => CanvasBlobTaskEffect::Retired,
        }
    }
}

pub(crate) fn canvas_to_blob_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CanvasToBlobArgs>(scope, &args) else {
        return;
    };
    // Quality is IDL `any`: only a primitive Number in [0, 1] selects a JPEG
    // quality. Objects, strings and non-finite values do not undergo ToNumber.
    let quality = args.get(2);
    let quality = quality
        .is_number()
        .then(|| quality.number_value(scope))
        .flatten();
    let canvas = args.this();
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let canvas_context = canvas
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    // Conversion may mutate the canvas. Snapshot after conversion and before
    // returning to script; later drawing or resizing cannot alter this export.
    let encode = CanvasBlobEncodeJob::new(
        canvas_like_pixels_copy(scope, canvas),
        &parsed.mime_type,
        quality,
    );
    let _ = unsafe { &mut *host_ptr }.queue_canvas_blob_serialization_task(
        scope,
        canvas_context,
        parsed.callback,
        encode,
    );
}
