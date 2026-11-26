use crate::{context_bootstrap::css_stylesheet_runtime::css_style_sheet_disabled, util::v8str};

pub(in crate::native_bridge) fn style_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(sheet) = args
        .this()
        .get(scope, v8str(scope, "sheet").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        let disabled = css_style_sheet_disabled(scope, sheet);
        rv.set(v8::Boolean::new(scope, disabled).into());
        return;
    }
    rv.set(v8::Boolean::new(scope, false).into());
}

pub(in crate::native_bridge) fn style_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let sheet = args
        .this()
        .get(scope, v8str(scope, "sheet").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    if let Some(sheet) = sheet {
        let _ = sheet.set(scope, v8str(scope, "disabled").into(), args.get(0));
    }
    rv.set_undefined();
}
