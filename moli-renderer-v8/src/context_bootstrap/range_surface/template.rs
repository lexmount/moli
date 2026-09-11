use super::{
    RANGE_WRAPPER_INTERNAL_FIELD_COUNT,
    construction::{range_constructor_callback, static_range_constructor_callback},
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::AbstractRange,
    constructor = "illegal",
    constructor_length = 0
)]
struct AbstractRangeTemplateDeclaration {}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::Range,
    constructor_callback = range_constructor_callback,
    constructor_length = 0
)]
struct RangeConstructorTemplateDeclaration {
    #[webapi(constant = "START_TO_START", value = 0u32)]
    start_to_start: (),

    #[webapi(constant = "START_TO_END", value = 1u32)]
    start_to_end: (),

    #[webapi(constant = "END_TO_END", value = 2u32)]
    end_to_end: (),

    #[webapi(constant = "END_TO_START", value = 3u32)]
    end_to_start: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::StaticRange,
    constructor_callback = static_range_constructor_callback,
    constructor_length = 1
)]
struct StaticRangeConstructorTemplateDeclaration {}

pub(in crate::context_bootstrap) fn build_abstract_range_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    AbstractRangeTemplateDeclaration::build(scope)
}

pub(in crate::context_bootstrap) fn build_range_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let template = RangeConstructorTemplateDeclaration::build(scope);
    let instance = template.instance_template(scope);
    let _ = instance.set_internal_field_count(RANGE_WRAPPER_INTERNAL_FIELD_COUNT);
    template
}

pub(in crate::context_bootstrap) fn build_static_range_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    StaticRangeConstructorTemplateDeclaration::build(scope)
}
