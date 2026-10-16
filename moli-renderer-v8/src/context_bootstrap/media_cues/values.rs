use crate::webidl;
use moli_webapi_declare::WebApiValue;

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "AutoKeyword", rename_all = "lowercase")]
enum AutoKeyword {
    Auto,
}

#[derive(Clone, Copy)]
pub(super) enum LineAndPositionSetting {
    Auto,
    Number(f64),
}

impl<'s> webidl::WebIdlConverter<'s> for LineAndPositionSetting {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        // A (double or enum) union selects the numeric member only for a
        // Number. Other values use the string member, including objects.
        if value.is_number() {
            webidl::convert::<webidl::Double>(scope, value, context)
                .map(|value| Self::Number(value.0))
        } else {
            webidl::convert::<webidl::EnumValue<AutoKeyword>>(scope, value, context)
                .map(|_| Self::Auto)
        }
    }
}

impl<'s> WebApiValue<'s> for LineAndPositionSetting {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        Some(match self {
            Self::Auto => crate::util::v8str(scope, "auto").into(),
            Self::Number(value) => v8::Number::new(scope, *value).into(),
        })
    }
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "DirectionSetting")]
pub(super) enum DirectionSetting {
    #[webidl(token = "")]
    Horizontal,
    #[webidl(token = "rl")]
    RightToLeft,
    #[webidl(token = "lr")]
    LeftToRight,
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "LineAlignSetting", rename_all = "lowercase")]
pub(super) enum LineAlignSetting {
    Start,
    Center,
    End,
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "PositionAlignSetting")]
pub(super) enum PositionAlignSetting {
    #[webidl(token = "line-left")]
    LineLeft,
    #[webidl(token = "center")]
    Center,
    #[webidl(token = "line-right")]
    LineRight,
    #[webidl(token = "auto")]
    Auto,
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "AlignSetting", rename_all = "lowercase")]
pub(super) enum AlignSetting {
    Start,
    Center,
    End,
    Left,
    Right,
}
