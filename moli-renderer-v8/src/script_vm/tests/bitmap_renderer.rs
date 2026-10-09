use super::*;

impl ScriptVm {
    // Register wrappers around objects produced by the real page task executor.
    // Author-created Proxy objects in the fixtures remain unregistered.
    pub(crate) fn register_bitmap_renderer_proxies_for_test(&mut self) -> anyhow::Result<()> {
        let context_ptr = &self.page_default_runtime.context as *const _;
        self.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            for (target_name, proxy_name) in [
                ("nativeRenderer", "rendererProxy"),
                ("nativeBitmap", "bitmapProxy"),
            ] {
                let key = crate::util::v8str(scope, target_name);
                let target =
                    v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap())
                        .unwrap();
                let handler = crate::util::new_null_prototype_object(scope);
                let proxy = v8::Proxy::new(scope, target, handler).unwrap();
                moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
                let key = crate::util::v8str(scope, proxy_name);
                assert_eq!(
                    global.create_data_property(scope, key.into(), proxy.into()),
                    Some(true)
                );
            }
            Ok(())
        })
    }
}
