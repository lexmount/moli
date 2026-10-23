use crate::webidl;

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "ScrollBehavior", rename_all = "kebab-case")]
enum ScrollBehavior {
    Auto,
    Instant,
    Smooth,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ScrollOptions")]
struct ScrollOptions {
    // The native scroll algorithm retains its existing immediate-scroll policy.
    #[webidl(name = "behavior", converter = "enum", default = ScrollBehavior::Auto)]
    _behavior: ScrollBehavior,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ScrollToOptions")]
struct ScrollToOptions {
    #[webidl(inherit)]
    _base: ScrollOptions,
    #[webidl(converter = "unrestricted_double")]
    left: Option<f64>,
    #[webidl(converter = "unrestricted_double")]
    top: Option<f64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "scroll")]
struct ScrollToArgs {
    #[webidl(dictionary)]
    options: ScrollToOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "scroll")]
struct ScrollNumericArgs {
    #[webidl(required, converter = "unrestricted_double")]
    x: f64,
    #[webidl(required, converter = "unrestricted_double")]
    y: f64,
}

pub(crate) struct ScrollCoordinates {
    left: Option<f64>,
    top: Option<f64>,
}

impl ScrollCoordinates {
    pub(crate) fn resolve(self, current_x: f64, current_y: f64, relative: bool) -> (f64, f64) {
        let normalize = |value: f64| if value.is_finite() { value } else { 0.0 };
        let left = self.left.map(normalize);
        let top = self.top.map(normalize);
        if relative {
            (
                current_x + left.unwrap_or(0.0),
                current_y + top.unwrap_or(0.0),
            )
        } else {
            (left.unwrap_or(current_x), top.unwrap_or(current_y))
        }
    }
}

pub(crate) fn coordinates<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<ScrollCoordinates> {
    // The legacy scroll bindings silently cancel failed argument conversion.
    // Keep the catch here so native layout failures remain visible to callers.
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let parsed = try_coordinates(&mut scope, args);
    if scope.has_caught() {
        scope.reset();
        return None;
    }
    parsed.ok()
}

/// Convert shared ScrollToOptions without consuming a pending exception.
/// Window promise bindings propagate conversion failures in the callee realm.
pub(crate) fn try_coordinates<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<ScrollCoordinates, webidl::WebIdlError> {
    if args.length() >= 2 {
        webidl::try_parse_args::<ScrollNumericArgs>(scope, args).map(|args| ScrollCoordinates {
            left: Some(args.x),
            top: Some(args.y),
        })
    } else {
        webidl::try_parse_args::<ScrollToArgs>(scope, args).map(|args| ScrollCoordinates {
            left: args.options.left,
            top: args.options.top,
        })
    }
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "ScrollLogicalPosition", rename_all = "kebab-case")]
pub(crate) enum ScrollLogicalPosition {
    Start,
    Center,
    End,
    Nearest,
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "ScrollIntoViewContainer", rename_all = "kebab-case")]
enum ScrollIntoViewContainer {
    All,
    Nearest,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ScrollIntoViewOptions")]
struct ScrollIntoViewOptions {
    #[webidl(inherit)]
    _base: ScrollOptions,
    #[webidl(converter = "enum", default = ScrollLogicalPosition::Start)]
    block: ScrollLogicalPosition,
    // The native bubbling algorithm retains its existing all-containers policy.
    #[webidl(name = "container", converter = "enum", default = ScrollIntoViewContainer::All)]
    _container: ScrollIntoViewContainer,
    #[webidl(converter = "enum", default = ScrollLogicalPosition::Nearest)]
    inline: ScrollLogicalPosition,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.scrollIntoView")]
struct ScrollIntoViewArgs {
    #[webidl(dictionary)]
    options: ScrollIntoViewOptions,
}

pub(crate) fn into_view_alignments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(ScrollLogicalPosition, ScrollLogicalPosition)> {
    let value = args.get(0);
    if !value.is_null_or_undefined() && !value.is_object() {
        return Some((
            ScrollLogicalPosition::Nearest,
            if value.boolean_value(scope) {
                ScrollLogicalPosition::Start
            } else {
                ScrollLogicalPosition::End
            },
        ));
    }
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let parsed = webidl::try_parse_args::<ScrollIntoViewArgs>(&mut scope, args);
    if scope.has_caught() {
        scope.reset();
        return None;
    }
    parsed
        .ok()
        .map(|args| (args.options.inline, args.options.block))
}
