use super::WindowLazySurface;
use anyhow::{Result, anyhow};

pub(super) fn build_window_lazy_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
) -> Result<v8::Local<'s, v8::Value>> {
    let value: v8::Local<'s, v8::Value> = match surface {
        WindowLazySurface::Navigator => {
            super::super::navigator_runtime::build_window_navigator_for_receiver(scope, window)?
                .into()
        }
        WindowLazySurface::External => super::super::external::build_external_object(scope)
            .ok_or_else(|| anyhow!("failed to create External"))?
            .into(),
        WindowLazySurface::Performance => {
            super::super::performance_runtime::build_window_performance_for_receiver(scope, window)
                .into()
        }
        WindowLazySurface::CustomElements => {
            crate::custom_elements::build_custom_elements_registry_for_window(scope, window)?.into()
        }
        WindowLazySurface::Screen => {
            super::super::navigator_runtime::build_window_screen(scope)?.into()
        }
        WindowLazySurface::Crypto => {
            super::super::crypto::build_window_crypto_for_receiver(scope, window)?.into()
        }
        WindowLazySurface::VisualViewport => {
            super::super::navigator_runtime::build_window_visual_viewport(scope, window)?.into()
        }
        WindowLazySurface::SpeechSynthesis => {
            super::super::speech_synthesis::build_window_speech_synthesis(scope)?.into()
        }
    };
    Ok(value)
}

pub(super) fn finish_window_lazy_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    surface: WindowLazySurface,
    value: v8::Local<'s, v8::Value>,
) -> Result<()> {
    if surface != WindowLazySurface::Performance {
        return Ok(());
    }
    let performance = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| anyhow!("Performance builder returned a non-object value"))?;
    super::super::performance_runtime::finish_window_performance_materialization(
        scope,
        window,
        performance,
    );
    Ok(())
}
