use crate::webidl;

pub(super) fn canvas_unrestricted_double_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> Option<f64> {
    webidl::argument::<webidl::UnrestrictedDouble>(
        scope,
        args,
        index,
        webidl::Context::argument(prefix, (index + 1) as usize),
    )
    .ok()
    .map(f64::from)
}

pub(super) fn canonical_canvas_fill_style(raw: &str) -> Option<String> {
    moli_canvas::canonicalize_fill_style(raw).or_else(|| {
        let [red, green, blue, alpha] =
            moli_css_parse::parse_absolute_css_color_to_srgb_bytes(raw)?;
        // Reuse the canvas serializer so both painting and the style getter
        // consume the same canonical sRGB value, including alpha.
        moli_canvas::canonicalize_fill_style(&format!("#{red:02x}{green:02x}{blue:02x}{alpha:02x}"))
    })
}

pub(super) use moli_webapi::canvas_context_values::init_canvas_like_context_object;
