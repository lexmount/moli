//! Analyser views are copies of native graph history, never fabricated samples.
use super::*;
use web_audio_api::node::AnalyserNode;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AnalyserNode)]
struct AnalyserObject {
    #[webapi(method, length = 1, callback = audio_node_connect_callback)]
    connect: (),
    #[webapi(method, length = 0, callback = audio_node_disconnect_callback)]
    disconnect: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AnalyserNode, enumerable)]
struct AnalyserPrototype {
    #[webapi(accessor_property = "fftSize", getter = fft_size, setter = set_fft_size)]
    fft_size: (),
    #[webapi(accessor_property = "frequencyBinCount", getter = frequency_bin_count)]
    frequency_bin_count: (),
    #[webapi(accessor_property, getter = min_decibels, setter = set_min_decibels)]
    min_decibels: (),
    #[webapi(accessor_property, getter = max_decibels, setter = set_max_decibels)]
    max_decibels: (),
    #[webapi(accessor_property, getter = smoothing_time_constant, setter = set_smoothing_time_constant)]
    smoothing_time_constant: (),
    #[webapi(method, length = 1, callback = get_float_frequency_data)]
    get_float_frequency_data: (),
    #[webapi(method, length = 1, callback = get_float_time_domain_data)]
    get_float_time_domain_data: (),
    #[webapi(method, length = 1, callback = get_byte_frequency_data)]
    get_byte_frequency_data: (),
    #[webapi(method, length = 1, callback = get_byte_time_domain_data)]
    get_byte_time_domain_data: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    AnalyserPrototype::initialize_prototype_template(scope, template.prototype_template(scope));
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let native = backend::create_node(scope, context, NodeKind::Analyser);
    let object = AnalyserObject::new()
        .bind(scope)
        .expect("AnalyserNode should bind");
    graph::initialize_node(scope, object, context);
    backend::initialize(scope, object, State::Node(native));
    object
}

fn with_analyser<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    operation: impl FnOnce(&mut AnalyserNode) -> T,
) -> Option<T> {
    if let Some(state) = backend::get(scope, object)
        && let State::Node(Node::Analyser(node)) = &mut *state.borrow_mut()
    {
        return Some(operation(node));
    }
    throw_type_error(scope, "Illegal invocation: expected an AnalyserNode.");
    None
}

fn fft_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(size) = with_analyser(scope, args.this(), |node| node.fft_size()) {
        rv.set_uint32(size as u32);
    }
}

fn frequency_bin_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(size) = with_analyser(scope, args.this(), |node| node.frequency_bin_count()) {
        rv.set_uint32(size as u32);
    }
}

fn set_fft_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if with_analyser(scope, args.this(), |_| ()).is_none() {
        return;
    }
    let Some(size) = args.get(0).uint32_value(scope) else {
        return;
    };
    if !(32..=32768).contains(&size) || !size.is_power_of_two() {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "AnalyserNode.fftSize must be a power of two between 32 and 32768.",
        );
        return;
    }
    with_analyser(scope, args.this(), |node| node.set_fft_size(size as usize));
}

macro_rules! configuration {
    ($get:ident, $set:ident, $valid:expr) => {
        fn $get<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'s, v8::Value>,
        ) {
            if let Some(value) = with_analyser(scope, args.this(), |node| node.$get()) {
                rv.set(v8::Number::new(scope, value).into());
            }
        }
        fn $set<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            _rv: v8::ReturnValue<'s, v8::Value>,
        ) {
            if with_analyser(scope, args.this(), |_| ()).is_none() {
                return;
            }
            let Some(value) = args.get(0).number_value(scope) else {
                return;
            };
            if !value.is_finite() {
                throw_type_error(scope, "AnalyserNode configuration must be finite.");
                return;
            }
            let valid = with_analyser(scope, args.this(), |node| {
                if !($valid)(node, value) {
                    return false;
                }
                node.$set(value);
                true
            });
            if valid == Some(false) {
                throw_dom_exception(
                    scope,
                    "IndexSizeError",
                    1,
                    "AnalyserNode configuration is out of range.",
                );
            }
        }
    };
}
configuration!(
    min_decibels,
    set_min_decibels,
    |node: &AnalyserNode, value| value < node.max_decibels()
);
configuration!(
    max_decibels,
    set_max_decibels,
    |node: &AnalyserNode, value| value > node.min_decibels()
);
configuration!(
    smoothing_time_constant,
    set_smoothing_time_constant,
    |_: &AnalyserNode, value| (0.0..=1.0).contains(&value)
);

enum Data {
    FloatFrequency,
    FloatTime,
    ByteFrequency,
    ByteTime,
}
macro_rules! data_method {
    ($name:ident, $kind:ident) => {
        fn $name<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            _rv: v8::ReturnValue<'s, v8::Value>,
        ) {
            copy_data(scope, &args, Data::$kind);
        }
    };
}
data_method!(get_float_frequency_data, FloatFrequency);
data_method!(get_float_time_domain_data, FloatTime);
data_method!(get_byte_frequency_data, ByteFrequency);
data_method!(get_byte_time_domain_data, ByteTime);

fn copy_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    kind: Data,
) {
    let Some(size) = with_analyser(scope, args.this(), |node| node.fft_size()) else {
        return;
    };
    let array: Option<v8::Local<v8::TypedArray>> = match kind {
        Data::FloatFrequency | Data::FloatTime => {
            v8::Local::<v8::Float32Array>::try_from(args.get(0))
                .ok()
                .map(Into::into)
        }
        Data::ByteFrequency | Data::ByteTime => v8::Local::<v8::Uint8Array>::try_from(args.get(0))
            .ok()
            .map(Into::into),
    };
    let Some(array) = array else {
        throw_type_error(
            scope,
            "AnalyserNode requires the matching Float32Array or Uint8Array.",
        );
        return;
    };
    if let Some(store) = array.get_backing_store()
        && (store.is_shared() || store.is_resizable_by_user_javascript())
    {
        throw_type_error(
            scope,
            "AnalyserNode data requires a non-shared, fixed-length buffer.",
        );
        return;
    }
    let count = match kind {
        Data::FloatFrequency | Data::ByteFrequency => size / 2,
        _ => size,
    };
    let count = count.min(array.length());
    let Some(values) = with_analyser(scope, args.this(), |node| match kind {
        Data::FloatFrequency | Data::FloatTime => {
            let mut values = vec![0.0_f32; count];
            match kind {
                Data::FloatFrequency => node.get_float_frequency_data(&mut values),
                _ => node.get_float_time_domain_data(&mut values),
            }
            values.into_iter().map(f64::from).collect::<Vec<_>>()
        }
        Data::ByteFrequency | Data::ByteTime => {
            let mut values = vec![0_u8; count];
            match kind {
                Data::ByteFrequency => node.get_byte_frequency_data(&mut values),
                _ => node.get_byte_time_domain_data(&mut values),
            }
            values.into_iter().map(f64::from).collect::<Vec<_>>()
        }
    }) else {
        return;
    };
    // Numeric indexed writes to a validated fixed typed array cannot invoke
    // page getters. The intrinsic length respects subarrays and detached views.
    for (index, value) in values.into_iter().enumerate() {
        let value = v8::Number::new(scope, value);
        if array.set_index(scope, index as u32, value.into()).is_none() {
            return;
        }
    }
}
