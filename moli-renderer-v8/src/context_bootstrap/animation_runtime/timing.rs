use super::*;
use crate::{
    animation_model::{Animation, AnimationInstance, AnimationKind, Easing},
    webidl,
};
use moli_animation::{CssTime, FillMode, KeyframeValue, PropertyKeyframes, duration};

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "OptionalEffectTiming")]
struct TimingOptions {
    #[webidl(converter = "double")]
    delay: Option<f64>,
    direction: Option<String>,
    #[webidl(converter = "raw")]
    duration: Option<EffectDuration>,
    easing: Option<String>,
    #[webidl(converter = "double")]
    end_delay: Option<f64>,
    fill: Option<String>,
    #[webidl(converter = "double")]
    iteration_start: Option<f64>,
    #[webidl(converter = "raw")]
    iterations: Option<webidl::UnrestrictedDouble>,
}

struct EffectDuration(f64);
impl<'s> webidl::WebIdlConverter<'s> for EffectDuration {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if value.is_string() {
            let text = webidl::convert::<webidl::DomString>(scope, value, context)?.0;
            if text == "auto" {
                return Ok(Self(0.0));
            }
            return Err(webidl::WebIdlError::custom_message(
                "duration must be a non-negative number or 'auto'",
            ));
        }
        let value = webidl::convert::<webidl::UnrestrictedDouble>(scope, value, context)?.0;
        if value.is_nan() || value < 0.0 {
            return Err(webidl::WebIdlError::custom_message(
                "duration must be non-negative",
            ));
        }
        Ok(Self(value / 1000.0))
    }
}

pub(super) fn parse_timing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    current: Option<v8::Local<'s, v8::Object>>,
) -> Option<Animation> {
    let options = if value.is_number() {
        let duration = match webidl::convert::<EffectDuration>(
            scope,
            value,
            webidl::Context::member("KeyframeEffect", "duration"),
        ) {
            Ok(value) => value,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        };
        TimingOptions {
            duration: Some(duration),
            ..Default::default()
        }
    } else {
        match webidl::convert::<webidl::Dictionary<TimingOptions>>(
            scope,
            value,
            webidl::Context::argument("KeyframeEffect", 3),
        ) {
            Ok(value) => value.0,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        }
    };
    let easing = if let Some(value) = options.easing {
        match Easing::parse(&value) {
            Some(easing) => Some(easing),
            None => {
                throw_type_error(scope, "Invalid easing function");
                return None;
            }
        }
    } else {
        None
    };
    if options
        .iterations
        .as_ref()
        .is_some_and(|v| v.0.is_nan() || v.0 < 0.0)
        || options.iteration_start.is_some_and(|v| v < 0.0)
    {
        throw_type_error(scope, "Iteration values must be non-negative");
        return None;
    }
    if options.direction.as_ref().is_some_and(|v| {
        !matches!(
            v.as_str(),
            "normal" | "reverse" | "alternate" | "alternate-reverse"
        )
    }) || options.fill.as_ref().is_some_and(|v| {
        !matches!(
            v.as_str(),
            "none" | "auto" | "backwards" | "forwards" | "both"
        )
    }) {
        throw_type_error(scope, "Invalid effect timing keyword");
        return None;
    }
    // Dictionary getters and string/number conversion may move this effect to
    // another Animation. Resolve its current native state only after all
    // conversions and validation, before committing any timing member.
    let animation = match current {
        Some(effect) => crate::animation_bindings::instance(scope, effect)?,
        None => AnimationInstance::new(
            AnimationKind::Script,
            String::new(),
            std::time::Instant::now(),
        )
        .shared(),
    };
    {
        let mut state = animation.borrow_mut();
        state
            .playback
            .preserve_current_time_on_timing_change(super::native::animation_time(scope));
        state.playback.has_effect = true;
        if let Some(v) = options.delay {
            state.timing_overrides.delay = true;
            state.playback.delay = CssTime::from_seconds(v / 1000.0).unwrap_or_default();
        }
        if let Some(v) = options.duration {
            state.timing_overrides.duration = true;
            state.playback.duration = duration(v.0);
            state.playback.duration_infinite = v.0.is_infinite();
        }
        if let Some(v) = options.iterations {
            state.timing_overrides.iterations = true;
            state.playback.iterations = v.0;
        }
        if let Some(v) = options.end_delay {
            state.playback.end_delay = CssTime::from_seconds(v / 1000.0).unwrap_or_default();
        }
        if let Some(v) = options.iteration_start {
            state.playback.iteration_start = v;
        }
        if let Some(v) = options.direction {
            state.timing_overrides.direction = true;
            state.playback.direction = v;
        }
        if let Some(v) = options.fill {
            state.timing_overrides.fill = true;
            state.playback.fill = FillMode::parse(&v);
        }
        if let Some(v) = easing {
            state.timing_overrides.easing = true;
            state.easing = v;
        }
    }
    Some(animation)
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "BaseKeyframe")]
struct KeyframeMetadata {
    composite: Option<String>,
    easing: Option<String>,
    #[webidl(nullable, converter = "double")]
    offset: Option<f64>,
}

pub(super) fn keyframes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Vec<PropertyKeyframes>> {
    if value.is_null_or_undefined() {
        return Some(Vec::new());
    }
    let object = match v8::Local::<v8::Object>::try_from(value) {
        Ok(object) => object,
        Err(_) => {
            throw_type_error(scope, "Keyframes must be an object");
            return None;
        }
    };
    let mut result: Vec<PropertyKeyframes> = Vec::new();
    fn add(
        result: &mut Vec<PropertyKeyframes>,
        property: String,
        offset: f64,
        value: String,
        easing: Option<String>,
    ) {
        let index = result
            .iter()
            .position(|p| p.property == property)
            .unwrap_or_else(|| {
                result.push(PropertyKeyframes {
                    property,
                    values: Vec::new(),
                });
                result.len() - 1
            });
        result[index].values.push(KeyframeValue {
            offset,
            value: Some(value),
            easing,
        });
    }
    if let Ok(array) = v8::Local::<v8::Array>::try_from(object) {
        let count = array.length();
        let mut inputs = Vec::with_capacity(count as usize);
        for index in 0..count {
            let frame = match v8::Local::<v8::Object>::try_from(array.get_index(scope, index)?) {
                Ok(frame) => frame,
                Err(_) => {
                    throw_type_error(scope, "Keyframes must be objects");
                    return None;
                }
            };
            let metadata = match webidl::convert::<webidl::Dictionary<KeyframeMetadata>>(
                scope,
                frame.into(),
                webidl::Context::argument("KeyframeEffect", 2),
            ) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return None;
                }
            };
            if metadata
                .composite
                .as_deref()
                .is_some_and(|v| !matches!(v, "auto" | "replace" | "add" | "accumulate"))
            {
                throw_type_error(scope, "Invalid keyframe composite operation");
                return None;
            }
            let easing = match metadata.easing.as_deref() {
                None | Some("linear") => None,
                Some(css) => {
                    if Easing::parse(css).is_none() {
                        throw_type_error(scope, "Invalid keyframe easing");
                        return None;
                    }
                    Some(css.to_owned())
                }
            };
            let names = frame.get_own_property_names(scope, Default::default())?;
            let mut properties = Vec::new();
            for j in 0..names.length() {
                let key = names.get_index(scope, j)?;
                let property = key.to_string(scope)?.to_rust_string_lossy(scope);
                if matches!(property.as_str(), "offset" | "easing" | "composite") {
                    continue;
                }
                let property = moli_css_parse::canonical_style_property_identifier(&property);
                let text = frame
                    .get(scope, key)?
                    .to_string(scope)?
                    .to_rust_string_lossy(scope);
                properties.push((property, text));
            }
            inputs.push((metadata.offset, easing, properties));
        }
        let offsets = match moli_animation::resolve_keyframe_offsets(
            &inputs.iter().map(|i| i.0).collect::<Vec<_>>(),
        ) {
            Some(offsets) => offsets,
            None => {
                throw_type_error(
                    scope,
                    "Keyframe offsets must be ordered and lie between 0 and 1",
                );
                return None;
            }
        };
        for ((_, easing, properties), offset) in inputs.into_iter().zip(offsets) {
            for (property, value) in properties {
                add(&mut result, property, offset, value, easing.clone());
            }
        }
    } else {
        fn list<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            value: v8::Local<'s, v8::Value>,
        ) -> Option<Vec<v8::Local<'s, v8::Value>>> {
            if value.is_undefined() {
                return Some(Vec::new());
            }
            if let Ok(array) = v8::Local::<v8::Array>::try_from(value) {
                (0..array.length())
                    .map(|i| array.get_index(scope, i))
                    .collect()
            } else {
                Some(vec![value])
            }
        }
        let raw_easing = object.get(scope, v8str(scope, "easing").into())?;
        let mut easings = Vec::new();
        for value in list(scope, raw_easing)? {
            let css = value.to_string(scope)?.to_rust_string_lossy(scope);
            if Easing::parse(&css).is_none() {
                throw_type_error(scope, "Invalid keyframe easing");
                return None;
            }
            easings.push((css != "linear").then_some(css));
        }
        let raw_offsets = object.get(scope, v8str(scope, "offset").into())?;
        let mut offsets = Vec::new();
        for value in list(scope, raw_offsets)? {
            if value.is_null_or_undefined() {
                offsets.push(None);
            } else {
                let value = match webidl::convert::<webidl::Double>(
                    scope,
                    value,
                    webidl::Context::member("PropertyIndexedKeyframe", "offset"),
                ) {
                    Ok(value) => value.0,
                    Err(error) => {
                        webidl::throw_error(scope, &error);
                        return None;
                    }
                };
                offsets.push(Some(value));
            }
        }
        let names = object.get_own_property_names(scope, Default::default())?;
        for i in 0..names.length() {
            let key = names.get_index(scope, i)?;
            let property = key.to_string(scope)?.to_rust_string_lossy(scope);
            if matches!(property.as_str(), "offset" | "easing" | "composite") {
                continue;
            }
            let property = moli_css_parse::canonical_style_property_identifier(&property);
            let values = object.get(scope, key)?;
            if let Ok(values) = v8::Local::<v8::Array>::try_from(values) {
                let count = values.length();
                for j in 0..count {
                    let text = values
                        .get_index(scope, j)?
                        .to_string(scope)?
                        .to_rust_string_lossy(scope);
                    add(
                        &mut result,
                        property.clone(),
                        if count <= 1 {
                            1.0
                        } else {
                            f64::from(j) / f64::from(count - 1)
                        },
                        text,
                        None,
                    );
                }
            } else {
                add(
                    &mut result,
                    property,
                    1.0,
                    values.to_string(scope)?.to_rust_string_lossy(scope),
                    None,
                );
            }
        }
        let mut merged = result
            .iter()
            .flat_map(|p| p.values.iter().map(|v| v.offset))
            .collect::<Vec<_>>();
        merged.sort_by(f64::total_cmp);
        merged.dedup();
        let mut input_offsets = if offsets.is_empty() {
            merged.iter().copied().map(Some).collect::<Vec<_>>()
        } else {
            // Supplied metadata replaces the merged offsets. An omitted tail
            // remains unspecified and is distributed between its new bounds.
            vec![None; merged.len()]
        };
        for (target, value) in input_offsets.iter_mut().zip(offsets) {
            *target = value;
        }
        let Some(resolved) = moli_animation::resolve_keyframe_offsets(&input_offsets) else {
            throw_type_error(
                scope,
                "Keyframe offsets must be ordered and lie between 0 and 1",
            );
            return None;
        };
        for property in &mut result {
            for value in &mut property.values {
                let index = merged
                    .binary_search_by(|offset| offset.total_cmp(&value.offset))
                    .ok()?;
                value.offset = resolved[index];
                value.easing = (!easings.is_empty())
                    .then(|| easings[index % easings.len()].clone())
                    .flatten();
            }
        }
    }
    for property in &mut result {
        property
            .values
            .sort_by(|a, b| a.offset.total_cmp(&b.offset));
    }
    Some(result)
}
