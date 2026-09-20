use super::*;

pub(in crate::context_bootstrap) fn sync_local_document_front_from_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) {
    if runtime_window_is_global(scope, owner) {
        return;
    }
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, owner)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(location) = window_location_for_holder(scope, owner)
        && let Some(href) = super::super::location_runtime::location_href_slot(scope, location)
        && let Ok(url) = url::Url::parse(&href)
    {
        let _ =
            unsafe { &mut *host_ptr }.set_lightweight_popup_same_document_url(scope, popup_id, url);
    }
    let Some(document) = owner
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    super::sync_document_location_runtime_state_from_window(scope, document, owner);
}
