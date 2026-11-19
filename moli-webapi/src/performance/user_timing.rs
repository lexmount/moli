use super::PerformanceEntryName;
use moli_webidl as webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.mark")]
pub struct PerformanceMarkArgs {
    #[webidl(required, converter = "raw")]
    pub name: PerformanceEntryName,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PerformanceMark")]
pub struct PerformanceMarkConstructorArgs {
    #[webidl(required, converter = "raw")]
    pub name: PerformanceEntryName,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PerformanceMarkOptions")]
pub struct PerformanceMarkOptions<'s> {
    #[webidl(converter = "raw")]
    pub detail: Option<v8::Local<'s, v8::Value>>,
    #[webidl(name = "startTime", converter = "double")]
    pub start_time: Option<f64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.clearMarks")]
pub struct PerformanceClearMarksArgs {
    #[webidl(converter = "raw")]
    pub name: Option<PerformanceEntryName>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.measure")]
pub struct PerformanceMeasureArgs<'s> {
    #[webidl(required, converter = "raw")]
    pub name: PerformanceEntryName,
    #[webidl(
        converter = "raw",
        default = MeasureStartOrOptions::Dictionary(PerformanceMeasureOptions::default())
    )]
    pub start_or_options: MeasureStartOrOptions<'s>,
    #[webidl(converter = "raw")]
    pub end_mark: Option<PerformanceEntryName>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PerformanceMeasureOptions")]
pub struct PerformanceMeasureOptions<'s> {
    #[webidl(converter = "raw")]
    pub detail: Option<v8::Local<'s, v8::Value>>,
    #[webidl(converter = "double")]
    pub duration: Option<f64>,
    #[webidl(converter = "raw")]
    pub end: Option<MeasureBoundary>,
    #[webidl(converter = "raw")]
    pub start: Option<MeasureBoundary>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.clearMeasures")]
pub struct PerformanceClearMeasuresArgs {
    #[webidl(converter = "raw")]
    pub name: Option<PerformanceEntryName>,
}

pub enum MeasureStartOrOptions<'s> {
    Mark(PerformanceEntryName),
    Dictionary(PerformanceMeasureOptions<'s>),
}

pub enum MeasureBoundary {
    Mark(PerformanceEntryName),
    Timestamp(f64),
}

impl<'s> webidl::WebIdlConverter<'s> for MeasureStartOrOptions<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if value.is_object() || value.is_null_or_undefined() {
            webidl::parse_dictionary::<PerformanceMeasureOptions>(scope, value, context)
                .map(|options| Self::Dictionary(options.unwrap_or_default()))
        } else {
            webidl::convert::<PerformanceEntryName>(scope, value, context).map(Self::Mark)
        }
    }
}

impl<'s> webidl::WebIdlConverter<'s> for MeasureBoundary {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if value.is_number() {
            webidl::convert::<webidl::Double>(scope, value, context)
                .map(|time| Self::Timestamp(time.0))
        } else {
            webidl::convert::<PerformanceEntryName>(scope, value, context).map(Self::Mark)
        }
    }
}

pub fn parse_performance_mark_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    context: webidl::Context,
) -> Option<PerformanceMarkOptions<'s>> {
    match webidl::dictionary_arg(args, 1, context) {
        Ok(Some(options)) => {
            match webidl::parse_dictionary_object::<PerformanceMarkOptions>(scope, options) {
                Ok(options) => Some(options),
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    None
                }
            }
        }
        Ok(None) => Some(PerformanceMarkOptions::default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}
