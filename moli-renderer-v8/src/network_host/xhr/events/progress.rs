use super::super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct ProgressEventTargetDeclaration<'scope> {
    target: v8::Local<'scope, v8::Object>,
    current_target: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct ProgressEventInitDeclaration {
    length_computable: bool,
    loaded: f64,
    total: f64,
}

pub(crate) fn make_progress_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    target: v8::Local<'_, v8::Object>,
    length_computable: bool,
    loaded: f64,
    total: f64,
) -> v8::Local<'s, v8::Object> {
    let target = local_object_in_scope(scope, target);
    let event = match new_progress_event_object(scope, event_type, length_computable, loaded, total)
    {
        Some(event) => event,
        None => ProgressEventTargetDeclaration::new(target, target)
            .bind(scope)
            .expect("XHR progress event target declaration should bind"),
    };
    ProgressEventTargetDeclaration::new(target, target)
        .initialize(scope, event)
        .expect("XHR progress event target declaration should initialize");
    event
}

fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

fn new_progress_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    length_computable: bool,
    loaded: f64,
    total: f64,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let ctor = global
        .get(scope, v8str(scope, "ProgressEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let init = progress_event_init(scope, length_computable, loaded, total);
    let event_type = v8_string(scope, event_type)?;
    ctor.new_instance(scope, &[event_type.into(), init.into()])
}

fn progress_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    length_computable: bool,
    loaded: f64,
    total: f64,
) -> v8::Local<'s, v8::Object> {
    ProgressEventInitDeclaration::new(length_computable, loaded, total)
        .bind(scope)
        .expect("ProgressEvent init declaration should bind")
}
