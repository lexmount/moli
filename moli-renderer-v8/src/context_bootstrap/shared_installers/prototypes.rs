use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(record)]
struct InterfacePrototypeTagDeclaration<'tag> {
    #[webapi(to_string_tag)]
    tag: &'tag str,
}

pub(in crate::context_bootstrap) fn install_to_string_tag(
    scope: &mut v8::PinScope<'_, '_>,
    global: v8::Local<'_, v8::Object>,
    ctor_name: &'static str,
    tag: &'static str,
) {
    let Some(ctor_val) = global.get(scope, v8str(scope, ctor_name).into()) else {
        return;
    };
    let Ok(ctor) = v8::Local::<v8::Object>::try_from(ctor_val) else {
        return;
    };
    let Some(proto_val) = ctor.get(scope, v8str(scope, "prototype").into()) else {
        return;
    };
    let Ok(proto) = v8::Local::<v8::Object>::try_from(proto_val) else {
        return;
    };
    let _ = InterfacePrototypeTagDeclaration::new(tag).initialize(scope, proto);
}
