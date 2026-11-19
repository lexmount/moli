use moli_webapi::webassembly::{self, StreamingResponse};

pub(crate) use webassembly::{
    capture_webassembly_default_prototypes, mark_module_instantiation_exceeds_v8_limit,
    module_instantiation_exceeds_v8_limit, set_current_context_webassembly_default_prototype,
    webassembly_default_prototype_for_context,
};

struct NetworkResponse;

impl StreamingResponse for NetworkResponse {
    fn consume_for_compilation<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        crate::network_host::consume_webassembly_streaming_response_value(scope, value)
    }
}

pub(super) fn install_webassembly_runtime_extensions(
    scope: &mut v8::PinScope<'_, '_>,
) -> anyhow::Result<()> {
    webassembly::install_webassembly_runtime_extensions::<NetworkResponse>(scope)
}
