use crate::web_api_interfaces;
/// A closed native window for binding tests, never a production PiP request.
pub(crate) fn closed_window_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    #[derive(Default, moli_webapi_declare::WebApiObject)]
    #[webapi(interface = web_api_interfaces::PictureInPictureWindow)]
    struct ClosedWindow {
        #[webapi(slot = moli_webapi::event_target::SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
        event_listeners: (),
    }
    ClosedWindow::default()
        .bind(scope)
        .expect("closed native PiP window")
}
