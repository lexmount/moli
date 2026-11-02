use super::{Input, NativeRegExp, RegExpCreationFlags, v8_string};
use urlpattern::{
    Error, RegexSyntax, UrlPatternInit, UrlPatternOptions, component::Component, quirks,
    regexp::RegExp,
};

// Preserve ECMAScript source without validating it against Rust regex syntax.
// Both protocol inference and final component compilation use bounded V8 regex.
struct PatternRegExpSource(String);

impl RegExp for PatternRegExpSource {
    fn syntax() -> RegexSyntax {
        RegexSyntax::EcmaScript
    }

    fn parse(pattern: &str, _: &str, _: bool) -> Result<Self, ()> {
        Ok(Self(pattern.to_owned()))
    }

    fn matches<'a>(&self, _: &'a str) -> Option<Vec<Option<&'a str>>> {
        unreachable!("browser pattern parsing supplies a scoped protocol matcher")
    }

    fn pattern_string(&self) -> &str {
        &self.0
    }
}

fn protocol_matches_special_scheme(
    scope: &mut v8::PinScope<'_, '_>,
    component: &Component<PatternRegExpSource>,
) -> Result<bool, Error> {
    const SPECIAL: [&str; 6] = ["ftp", "file", "http", "https", "ws", "wss"];
    if !matches!(
        component.matcher.inner,
        urlpattern::matcher::InnerMatcher::RegExp { .. }
    ) {
        return Ok(SPECIAL
            .into_iter()
            .any(|scheme| component.matcher.matches(scheme).is_some()));
    }
    let source = component.regexp.as_ref().map_err(|_| Error::RegExp(()))?;
    let source = v8_string(scope, &source.0).ok_or(Error::RegExp(()))?;
    let regexp = NativeRegExp::new(scope, source, RegExpCreationFlags::UNICODE_SETS)
        .ok_or(Error::RegExp(()))?;
    for scheme in SPECIAL {
        let subject = v8_string(scope, scheme).ok_or(Error::RegExp(()))?;
        if regexp.is_match(scope, subject).unwrap_or(false) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn parse(
    scope: &mut v8::PinScope<'_, '_>,
    input: Input,
    base: Option<&str>,
    ignore_case: bool,
) -> Result<quirks::UrlPattern, Error> {
    let mut matcher = |component: &Component<PatternRegExpSource>| {
        protocol_matches_special_scheme(scope, component)
    };
    let init = match input.library_input() {
        quirks::StringOrInit::String(value) => {
            let base = base.map(url::Url::parse).transpose().map_err(Error::Url)?;
            UrlPatternInit::parse_constructor_string_with_protocol_matcher::<PatternRegExpSource>(
                &value,
                base,
                &mut matcher,
            )?
        }
        value => quirks::process_construct_pattern_input(value, base)?,
    };
    quirks::parse_pattern_with_protocol_matcher::<PatternRegExpSource>(
        init,
        UrlPatternOptions {
            regex_syntax: RegexSyntax::EcmaScript,
            ignore_case,
        },
        &mut matcher,
    )
}
