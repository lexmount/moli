use super::WindowLazySurface;
use crate::util::get_private_value;

pub(crate) fn window_lazy_surface_diagnostics(
    scope: &mut v8::PinScope<'_, '_>,
) -> WindowLazySurfaceDiagnostics {
    let global = scope.get_current_context().global(scope);
    WindowLazySurfaceDiagnostics {
        external_materialized: private_object_present(scope, global, WindowLazySurface::External),
        navigator_materialized: private_object_present(scope, global, WindowLazySurface::Navigator),
        performance_materialized: private_object_present(
            scope,
            global,
            WindowLazySurface::Performance,
        ),
        custom_elements_materialized: private_object_present(
            scope,
            global,
            WindowLazySurface::CustomElements,
        ),
        screen_materialized: private_object_present(scope, global, WindowLazySurface::Screen),
        crypto_materialized: private_object_present(scope, global, WindowLazySurface::Crypto),
        visual_viewport_materialized: private_object_present(
            scope,
            global,
            WindowLazySurface::VisualViewport,
        ),
        speech_synthesis_materialized: private_object_present(
            scope,
            global,
            WindowLazySurface::SpeechSynthesis,
        ),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WindowLazySurfaceDiagnostics {
    pub(crate) external_materialized: bool,
    pub(crate) navigator_materialized: bool,
    pub(crate) performance_materialized: bool,
    pub(crate) custom_elements_materialized: bool,
    pub(crate) screen_materialized: bool,
    pub(crate) crypto_materialized: bool,
    pub(crate) visual_viewport_materialized: bool,
    pub(crate) speech_synthesis_materialized: bool,
}

fn private_object_present<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
) -> bool {
    get_private_value(scope, object, surface.slot())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .is_some()
}
