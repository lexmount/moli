//! Owned WebIDL values. No author object survives conversion or participates
//! in a later source-selection task or getConstraints() snapshot.

use crate::{util::v8str, webidl};
use webidl::{Context, WebIdlConverter, WebIdlError};

pub(super) trait Snapshot {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value>;
}

fn member<'s, T: Snapshot + ?Sized>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    value: Option<&T>,
) {
    if let Some(value) = value {
        let value = value.snapshot(scope);
        object.create_data_property(scope, v8str(scope, name).into(), value);
    }
}

impl Snapshot for webidl::DomString16 {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::String::new_from_two_byte(scope, &self.0, v8::NewStringType::Normal)
            .expect("converted constraint string")
            .into()
    }
}
impl Snapshot for webidl::Boolean {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Boolean::new(scope, self.0).into()
    }
}
impl Snapshot for webidl::ClampedUnsignedLong {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Number::new(scope, f64::from(self.0)).into()
    }
}
impl Snapshot for webidl::Double {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Number::new(scope, self.0).into()
    }
}
impl<T: Snapshot> Snapshot for Vec<T> {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        let values: Vec<_> = self.iter().map(|value| value.snapshot(scope)).collect();
        v8::Array::new_with_elements(scope, &values).into()
    }
}
impl<T: Snapshot> Snapshot for webidl::Sequence<T> {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        self.0.snapshot(scope)
    }
}

pub(super) trait Constraint: Snapshot {
    fn required(&self) -> bool;
}

#[derive(Clone, webidl::WebIdlDictionary)]
#[webidl(prefix = "Range")]
struct Range<T: for<'a> WebIdlConverter<'a, Options = ()>> {
    #[webidl(converter = "raw")]
    max: Option<T>,
    #[webidl(converter = "raw")]
    min: Option<T>,
}

#[derive(Clone, webidl::WebIdlDictionary)]
#[webidl(prefix = "ConstrainRange")]
struct NumericParameters<T: for<'a> WebIdlConverter<'a, Options = ()>> {
    #[webidl(inherit)]
    range: Range<T>,
    #[webidl(converter = "raw")]
    exact: Option<T>,
    #[webidl(converter = "raw")]
    ideal: Option<T>,
}

#[derive(Clone)]
enum Numeric<T: for<'a> WebIdlConverter<'a, Options = ()>> {
    Value(T),
    Parameters(NumericParameters<T>),
}

impl<'s, T: for<'a> WebIdlConverter<'a, Options = ()>> WebIdlConverter<'s> for Numeric<T> {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &(),
    ) -> Result<Self, WebIdlError> {
        if value.is_object() || value.is_null_or_undefined() {
            webidl::convert::<webidl::Dictionary<NumericParameters<T>>>(scope, value, context)
                .map(|value| Self::Parameters(value.0))
        } else {
            T::convert(scope, value, context, &()).map(Self::Value)
        }
    }
}
impl<T: for<'a> WebIdlConverter<'a, Options = ()> + Snapshot> Snapshot for Numeric<T> {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        match self {
            Self::Value(value) => value.snapshot(scope),
            Self::Parameters(value) => {
                let result = v8::Object::new(scope);
                member(scope, result, "max", value.range.max.as_ref());
                member(scope, result, "min", value.range.min.as_ref());
                member(scope, result, "exact", value.exact.as_ref());
                member(scope, result, "ideal", value.ideal.as_ref());
                result.into()
            }
        }
    }
}
impl<T: for<'a> WebIdlConverter<'a, Options = ()> + Snapshot> Constraint for Numeric<T> {
    fn required(&self) -> bool {
        matches!(self, Self::Parameters(value) if value.range.max.is_some() || value.range.min.is_some() || value.exact.is_some())
    }
}

#[derive(Clone)]
enum StringValue {
    String(webidl::DomString16),
    Sequence(Vec<webidl::DomString16>),
}
impl<'s> WebIdlConverter<'s> for StringValue {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &(),
    ) -> Result<Self, WebIdlError> {
        if let Some(sequence) = webidl::convert_optional_sequence::<webidl::DomString16>(
            scope,
            value,
            context,
            &Default::default(),
        )? {
            Ok(Self::Sequence(sequence.0))
        } else {
            webidl::convert::<webidl::DomString16>(scope, value, context).map(Self::String)
        }
    }
}
impl Snapshot for StringValue {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        match self {
            Self::String(value) => value.snapshot(scope),
            Self::Sequence(value) => value.snapshot(scope),
        }
    }
}

#[derive(Clone)]
enum BooleanOrString {
    Boolean(webidl::Boolean),
    String(webidl::DomString16),
}
impl<'s> WebIdlConverter<'s> for BooleanOrString {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &(),
    ) -> Result<Self, WebIdlError> {
        if value.is_boolean() {
            webidl::convert(scope, value, context).map(Self::Boolean)
        } else {
            webidl::convert(scope, value, context).map(Self::String)
        }
    }
}
impl Snapshot for BooleanOrString {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        match self {
            Self::Boolean(value) => value.snapshot(scope),
            Self::String(value) => value.snapshot(scope),
        }
    }
}

#[derive(Clone, webidl::WebIdlDictionary)]
#[webidl(prefix = "ConstrainParameters")]
struct Parameters<T: for<'a> WebIdlConverter<'a, Options = ()>> {
    #[webidl(converter = "raw")]
    exact: Option<T>,
    #[webidl(converter = "raw")]
    ideal: Option<T>,
}

#[derive(Clone)]
enum Simple<T: for<'a> WebIdlConverter<'a, Options = ()>> {
    Value(T),
    Parameters(Parameters<T>),
}
impl<'s, T: for<'a> WebIdlConverter<'a, Options = ()>> WebIdlConverter<'s> for Simple<T> {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &(),
    ) -> Result<Self, WebIdlError> {
        if value.is_object() || value.is_null_or_undefined() {
            webidl::convert::<webidl::Dictionary<Parameters<T>>>(scope, value, context)
                .map(|value| Self::Parameters(value.0))
        } else {
            T::convert(scope, value, context, &()).map(Self::Value)
        }
    }
}
impl<T: for<'a> WebIdlConverter<'a, Options = ()> + Snapshot> Snapshot for Simple<T> {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        match self {
            Self::Value(value) => value.snapshot(scope),
            Self::Parameters(value) => {
                let result = v8::Object::new(scope);
                member(scope, result, "exact", value.exact.as_ref());
                member(scope, result, "ideal", value.ideal.as_ref());
                result.into()
            }
        }
    }
}
impl<T: for<'a> WebIdlConverter<'a, Options = ()> + Snapshot> Constraint for Simple<T> {
    fn required(&self) -> bool {
        matches!(self, Self::Parameters(value) if value.exact.is_some())
    }
}

#[derive(Clone)]
struct StringConstraint(Simple<StringValue>);
impl<'s> WebIdlConverter<'s> for StringConstraint {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &(),
    ) -> Result<Self, WebIdlError> {
        if let Some(sequence) = webidl::convert_optional_sequence::<webidl::DomString16>(
            scope,
            value,
            context,
            &Default::default(),
        )? {
            Ok(Self(Simple::Value(StringValue::Sequence(sequence.0))))
        } else {
            Simple::convert(scope, value, context, &()).map(Self)
        }
    }
}
impl Snapshot for StringConstraint {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        self.0.snapshot(scope)
    }
}
impl Constraint for StringConstraint {
    fn required(&self) -> bool {
        match &self.0 {
            Simple::Parameters(Parameters {
                exact: Some(StringValue::Sequence(values)),
                ..
            }) => !values.is_empty(),
            _ => self.0.required(),
        }
    }
}

#[derive(Clone)]
enum PanTiltZoom {
    Boolean(webidl::Boolean),
    Numeric(Numeric<webidl::Double>),
}
impl<'s> WebIdlConverter<'s> for PanTiltZoom {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &(),
    ) -> Result<Self, WebIdlError> {
        if value.is_boolean() {
            webidl::convert(scope, value, context).map(Self::Boolean)
        } else {
            webidl::convert(scope, value, context).map(Self::Numeric)
        }
    }
}
impl Snapshot for PanTiltZoom {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        match self {
            Self::Boolean(value) => value.snapshot(scope),
            Self::Numeric(value) => value.snapshot(scope),
        }
    }
}
impl Constraint for PanTiltZoom {
    fn required(&self) -> bool {
        matches!(self, Self::Numeric(value) if value.required())
    }
}

#[derive(Clone, webidl::WebIdlDictionary)]
#[webidl(prefix = "Point2D")]
struct Point {
    #[webidl(converter = "double", default = 0.0)]
    x: f64,
    #[webidl(converter = "double", default = 0.0)]
    y: f64,
}
impl<'s> WebIdlConverter<'s> for Point {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &(),
    ) -> Result<Self, WebIdlError> {
        webidl::convert::<webidl::Dictionary<Self>>(scope, value, context).map(|value| value.0)
    }
}
impl Snapshot for Point {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        let result = v8::Object::new(scope);
        result.create_data_property(
            scope,
            v8str(scope, "x").into(),
            v8::Number::new(scope, self.x).into(),
        );
        result.create_data_property(
            scope,
            v8str(scope, "y").into(),
            v8::Number::new(scope, self.y).into(),
        );
        result.into()
    }
}
#[derive(Clone)]
enum PointConstraint {
    Sequence(webidl::Sequence<Point>),
    Parameters(Parameters<webidl::Sequence<Point>>),
}
impl<'s> WebIdlConverter<'s> for PointConstraint {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: Context,
        _options: &(),
    ) -> Result<Self, WebIdlError> {
        if let Some(sequence) =
            webidl::convert_optional_sequence::<Point>(scope, value, context, &Default::default())?
        {
            Ok(Self::Sequence(sequence))
        } else {
            webidl::convert::<webidl::Dictionary<Parameters<webidl::Sequence<Point>>>>(
                scope, value, context,
            )
            .map(|value| Self::Parameters(value.0))
        }
    }
}
impl Snapshot for PointConstraint {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        match self {
            Self::Sequence(value) => value.snapshot(scope),
            Self::Parameters(value) => {
                let result = v8::Object::new(scope);
                member(scope, result, "exact", value.exact.as_ref());
                member(scope, result, "ideal", value.ideal.as_ref());
                result.into()
            }
        }
    }
}
impl Constraint for PointConstraint {
    fn required(&self) -> bool {
        matches!(self, Self::Parameters(value) if value.exact.as_ref().is_some_and(|points| !points.0.is_empty()))
    }
}

// Partial dictionaries from Media Capture, Image Capture, Screen Capture and
// Media Capture Extensions are one dictionary, converted in member-name order.
// Keeping the union converters here leaves traversal and inherited dictionary
// ordering to the shared WebIDL derive machinery.
#[derive(Clone, Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaTrackConstraintSet")]
pub(super) struct ConstraintSet {
    #[webidl(converter = "raw")]
    aspect_ratio: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    auto_gain_control: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    background_blur: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    background_segmentation_mask: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    brightness: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    channel_count: Option<Numeric<webidl::ClampedUnsignedLong>>,
    #[webidl(converter = "raw")]
    color_temperature: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    contrast: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    cursor: Option<StringConstraint>,
    #[webidl(converter = "raw")]
    device_id: Option<StringConstraint>,
    #[webidl(converter = "raw")]
    display_surface: Option<StringConstraint>,
    #[webidl(converter = "raw")]
    echo_cancellation: Option<Simple<BooleanOrString>>,
    #[webidl(converter = "raw")]
    exposure_compensation: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    exposure_mode: Option<StringConstraint>,
    #[webidl(converter = "raw")]
    exposure_time: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    eye_gaze_correction: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    face_framing: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    facing_mode: Option<StringConstraint>,
    #[webidl(converter = "raw")]
    focus_distance: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    focus_mode: Option<StringConstraint>,
    #[webidl(converter = "raw")]
    frame_rate: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    gesture_reactions: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    group_id: Option<StringConstraint>,
    #[webidl(converter = "raw")]
    height: Option<Numeric<webidl::ClampedUnsignedLong>>,
    #[webidl(converter = "raw")]
    human_face_detection_mode: Option<StringConstraint>,
    #[webidl(converter = "raw")]
    iso: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    latency: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    logical_surface: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    noise_suppression: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    pan: Option<PanTiltZoom>,
    #[webidl(converter = "raw")]
    points_of_interest: Option<PointConstraint>,
    #[webidl(converter = "raw")]
    resize_mode: Option<StringConstraint>,
    #[webidl(converter = "raw")]
    restrict_own_audio: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    sample_rate: Option<Numeric<webidl::ClampedUnsignedLong>>,
    #[webidl(converter = "raw")]
    sample_size: Option<Numeric<webidl::ClampedUnsignedLong>>,
    #[webidl(converter = "raw")]
    saturation: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    sharpness: Option<Numeric<webidl::Double>>,
    #[webidl(converter = "raw")]
    suppress_local_audio_playback: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    tilt: Option<PanTiltZoom>,
    #[webidl(converter = "raw")]
    torch: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    voice_isolation: Option<Simple<webidl::Boolean>>,
    #[webidl(converter = "raw")]
    white_balance_mode: Option<StringConstraint>,
    #[webidl(converter = "raw")]
    width: Option<Numeric<webidl::ClampedUnsignedLong>>,
    #[webidl(converter = "raw")]
    zoom: Option<PanTiltZoom>,
}

impl ConstraintSet {
    pub(super) fn entries(&self) -> [(&'static str, Option<&dyn Constraint>); 44] {
        [
            (
                "aspectRatio",
                self.aspect_ratio.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "autoGainControl",
                self.auto_gain_control
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            (
                "backgroundBlur",
                self.background_blur.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "backgroundSegmentationMask",
                self.background_segmentation_mask
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            (
                "brightness",
                self.brightness.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "channelCount",
                self.channel_count.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "colorTemperature",
                self.color_temperature
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            (
                "contrast",
                self.contrast.as_ref().map(|v| v as &dyn Constraint),
            ),
            ("cursor", self.cursor.as_ref().map(|v| v as &dyn Constraint)),
            (
                "deviceId",
                self.device_id.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "displaySurface",
                self.display_surface.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "echoCancellation",
                self.echo_cancellation
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            (
                "exposureCompensation",
                self.exposure_compensation
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            (
                "exposureMode",
                self.exposure_mode.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "exposureTime",
                self.exposure_time.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "eyeGazeCorrection",
                self.eye_gaze_correction
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            (
                "faceFraming",
                self.face_framing.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "facingMode",
                self.facing_mode.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "focusDistance",
                self.focus_distance.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "focusMode",
                self.focus_mode.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "frameRate",
                self.frame_rate.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "gestureReactions",
                self.gesture_reactions
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            (
                "groupId",
                self.group_id.as_ref().map(|v| v as &dyn Constraint),
            ),
            ("height", self.height.as_ref().map(|v| v as &dyn Constraint)),
            (
                "humanFaceDetectionMode",
                self.human_face_detection_mode
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            ("iso", self.iso.as_ref().map(|v| v as &dyn Constraint)),
            (
                "latency",
                self.latency.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "logicalSurface",
                self.logical_surface.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "noiseSuppression",
                self.noise_suppression
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            ("pan", self.pan.as_ref().map(|v| v as &dyn Constraint)),
            (
                "pointsOfInterest",
                self.points_of_interest
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            (
                "resizeMode",
                self.resize_mode.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "restrictOwnAudio",
                self.restrict_own_audio
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            (
                "sampleRate",
                self.sample_rate.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "sampleSize",
                self.sample_size.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "saturation",
                self.saturation.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "sharpness",
                self.sharpness.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "suppressLocalAudioPlayback",
                self.suppress_local_audio_playback
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            ("tilt", self.tilt.as_ref().map(|v| v as &dyn Constraint)),
            ("torch", self.torch.as_ref().map(|v| v as &dyn Constraint)),
            (
                "voiceIsolation",
                self.voice_isolation.as_ref().map(|v| v as &dyn Constraint),
            ),
            (
                "whiteBalanceMode",
                self.white_balance_mode
                    .as_ref()
                    .map(|v| v as &dyn Constraint),
            ),
            ("width", self.width.as_ref().map(|v| v as &dyn Constraint)),
            ("zoom", self.zoom.as_ref().map(|v| v as &dyn Constraint)),
        ]
    }
}
impl Snapshot for ConstraintSet {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        let result = v8::Object::new(scope);
        for (name, value) in self.entries() {
            member(scope, result, name, value);
        }
        result.into()
    }
}

#[derive(Clone, Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaTrackConstraints")]
pub(super) struct Constraints {
    #[webidl(inherit)]
    pub(super) base: ConstraintSet,
    #[webidl(sequence, converter = "dictionary")]
    pub(super) advanced: Option<Vec<ConstraintSet>>,
}
impl Snapshot for Constraints {
    fn snapshot<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        let result = v8::Local::<v8::Object>::try_from(self.base.snapshot(scope))
            .expect("native constraint dictionary");
        member(scope, result, "advanced", self.advanced.as_ref());
        result.into()
    }
}
