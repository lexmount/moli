use super::*;

impl ScriptVm {
    pub(crate) fn register_offscreen_canvas_proxy_for_test(&mut self) -> anyhow::Result<()> {
        let context_ptr = &self.page_default_context as *const _;
        self.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = crate::util::v8str(scope, "nativeOffscreen");
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy)?;
            let key = crate::util::v8str(scope, "offscreenProxy");
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
            Ok(())
        })
    }
}
