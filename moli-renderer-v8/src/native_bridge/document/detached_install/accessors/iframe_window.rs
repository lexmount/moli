use crate::util::v8str;
use moli_webapi_declare::WebApiObject;
use url::Url;

use crate::native_bridge::document::set_document_associated_window;

use super::super::super::detached_owner_document_object;
use super::iframe_style::install_detached_iframe_get_computed_style;
use super::iframe_window_messaging::install_detached_iframe_window_messaging;

#[derive(WebApiObject)]
#[webapi(interface = "Object", own_to_string_tag = "Window")]
struct DetachedIframeWindowDeclaration<'scope> {
    #[webapi(data_property = "self", value = object)]
    self_value: (),
    #[webapi(data_property, value = object)]
    window: (),
    #[webapi(data_property, value = object)]
    frames: (),
    #[webapi(data_property)]
    parent: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    top: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    document: v8::Local<'scope, v8::Object>,
}

pub(super) fn build_detached_iframe_content_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iframe: v8::Local<'s, v8::Object>,
    document: v8::Local<'s, v8::Object>,
    base_url: &Url,
) -> Option<v8::Local<'s, v8::Object>> {
    let parent = detached_iframe_parent_window(scope, iframe)
        .unwrap_or_else(|| scope.get_current_context().global(scope));
    let top = parent
        .get(scope, v8str(scope, "top").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or(parent);
    let window = DetachedIframeWindowDeclaration::new(parent, top, document)
        .bind(scope)
        .ok()?;
    crate::context_bootstrap::mark_window_receiver(scope, window);
    set_document_associated_window(scope, document, window);
    crate::network_host::install_fetch_constructors_for_base_url(scope, window, base_url);
    install_detached_iframe_window_messaging(scope, window);
    install_detached_iframe_get_computed_style(scope, window, iframe);
    Some(window)
}

fn detached_iframe_parent_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iframe: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    detached_owner_document_object(scope, iframe)?
        .get(scope, v8str(scope, "defaultView").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}
