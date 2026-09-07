use super::{
    css_fontface_runtime::{
        font_face_load_callback, font_face_set_add_callback,
        font_face_set_add_event_listener_callback, font_face_set_check_callback,
        font_face_set_clear_callback, font_face_set_delete_callback,
        font_face_set_dispatch_event_callback, font_face_set_entries_callback,
        font_face_set_for_each_callback, font_face_set_has_callback, font_face_set_keys_callback,
        font_face_set_load_callback, font_face_set_remove_event_listener_callback,
        font_face_set_values_callback, install_font_face_set_event_handler_accessors,
        install_font_face_set_load_event_template_accessors,
        install_font_face_set_template_accessors, install_font_face_template_accessors,
        is_font_face,
    },
    specs::ConstructorSpec,
};
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FontFace", enumerable, receiver = is_font_face)]
struct FontFaceTemplateMethodsDeclaration {
    #[webapi(method, length = 0, callback = font_face_load_callback, returns_promise)]
    load: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FontFaceSet", enumerable)]
struct FontFaceSetTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = font_face_set_add_callback)]
    add: (),

    #[webapi(method, length = 1, callback = font_face_set_has_callback)]
    has: (),

    #[webapi(method, length = 1, callback = font_face_set_delete_callback)]
    r#delete: (),

    #[webapi(method, length = 0, callback = font_face_set_clear_callback)]
    clear: (),

    #[webapi(method, length = 1, callback = font_face_set_check_callback)]
    check: (),

    #[webapi(method, length = 1, callback = font_face_set_load_callback)]
    load: (),

    #[webapi(method, length = 0, callback = font_face_set_keys_callback)]
    keys: (),

    #[webapi(method, length = 0, callback = font_face_set_values_callback)]
    values: (),

    #[webapi(alias = "values", symbol = "iterator")]
    iterator: (),

    #[webapi(method, length = 0, callback = font_face_set_entries_callback)]
    entries: (),

    #[webapi(method, length = 1, callback = font_face_set_for_each_callback)]
    for_each: (),

    #[webapi(method, length = 2, callback = font_face_set_add_event_listener_callback)]
    add_event_listener: (),

    #[webapi(
        method,
        length = 2,
        callback = font_face_set_remove_event_listener_callback
    )]
    remove_event_listener: (),

    #[webapi(method, length = 1, callback = font_face_set_dispatch_event_callback)]
    dispatch_event: (),
}

pub(super) fn install_style_font_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    spec: ConstructorSpec,
) {
    match spec.name {
        "FontFace" => {
            let prototype = template.prototype_template(scope);
            FontFaceTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
            install_font_face_template_accessors(scope, template);
        }
        "FontFaceSet" => {
            let prototype = template.prototype_template(scope);
            FontFaceSetTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
            install_font_face_set_template_accessors(scope, template);
            install_font_face_set_event_handler_accessors(scope, template);
        }
        "FontFaceSetLoadEvent" => {
            install_font_face_set_load_event_template_accessors(scope, template);
        }
        _ => {}
    }
}
