use moli_html_input_temporal::{
    MS_PER_DAY, MS_PER_SECOND, MS_PER_WEEK, WEEK_INPUT_STEP_BASE, date_input_milliseconds,
    date_input_value_from_milliseconds, datetime_local_input_milliseconds,
    datetime_local_input_value_from_milliseconds, month_input_number,
    month_input_value_from_number, time_input_milliseconds, time_input_value_from_milliseconds,
    week_input_milliseconds, week_input_value_from_milliseconds,
};
use moli_html_input_type::InputType;

pub fn is_valid_number_input_value(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    if value.starts_with('+')
        || value.ends_with('.')
        || value.chars().any(|ch| ch.is_ascii_whitespace())
    {
        return false;
    }
    value.parse::<f64>().is_ok_and(f64::is_finite)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressElementValues {
    pub value: f64,
    pub max: f64,
    pub position: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeterGaugeRegion {
    Optimum,
    Suboptimum,
    EvenLessGood,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterElementValues {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub low: f64,
    pub high: f64,
    pub optimum: f64,
    pub position: f64,
    pub gauge_region: MeterGaugeRegion,
}

pub fn meter_element_values(
    value_attribute: Option<&str>,
    min_attribute: Option<&str>,
    max_attribute: Option<&str>,
    low_attribute: Option<&str>,
    high_attribute: Option<&str>,
    optimum_attribute: Option<&str>,
) -> MeterElementValues {
    let parse_attribute = |attribute: Option<&str>| {
        attribute
            .and_then(parse_html_floating_point_prefix)
            .filter(|value| value.is_finite())
    };
    let min = parse_attribute(min_attribute).unwrap_or(0.0);
    let max = parse_attribute(max_attribute).unwrap_or(1.0).max(min);
    let value = parse_attribute(value_attribute)
        .unwrap_or(0.0)
        .clamp(min, max);
    let low = parse_attribute(low_attribute)
        .unwrap_or(min)
        .clamp(min, max);
    let high = parse_attribute(high_attribute)
        .unwrap_or(max)
        .clamp(low, max);
    let optimum = parse_attribute(optimum_attribute)
        .unwrap_or((min + max) / 2.0)
        .clamp(min, max);
    let position = if max > min {
        (value - min) / (max - min)
    } else {
        0.0
    };
    let gauge_region = if optimum < low {
        if value <= low {
            MeterGaugeRegion::Optimum
        } else if value <= high {
            MeterGaugeRegion::Suboptimum
        } else {
            MeterGaugeRegion::EvenLessGood
        }
    } else if optimum > high {
        if value >= high {
            MeterGaugeRegion::Optimum
        } else if value >= low {
            MeterGaugeRegion::Suboptimum
        } else {
            MeterGaugeRegion::EvenLessGood
        }
    } else if value >= low && value <= high {
        MeterGaugeRegion::Optimum
    } else {
        MeterGaugeRegion::Suboptimum
    };
    MeterElementValues {
        value,
        min,
        max,
        low,
        high,
        optimum,
        position,
        gauge_region,
    }
}

pub fn progress_element_values(
    value_attribute: Option<&str>,
    max_attribute: Option<&str>,
) -> ProgressElementValues {
    let max = max_attribute
        .and_then(parse_html_floating_point_prefix)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);
    let value = value_attribute
        .and_then(parse_html_floating_point_prefix)
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .clamp(0.0, max);
    let position = if value_attribute.is_some() {
        value / max
    } else {
        -1.0
    };
    ProgressElementValues {
        value,
        max,
        position,
    }
}

pub fn parse_html_floating_point_prefix(value: &str) -> Option<f64> {
    let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let bytes = value.as_bytes();
    let mut end = 0usize;
    if matches!(bytes.get(end), Some(b'+') | Some(b'-')) {
        end += 1;
    }
    let mut saw_digit = false;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        saw_digit = true;
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            saw_digit = true;
            end += 1;
        }
    }
    if !saw_digit {
        return None;
    }
    if matches!(bytes.get(end), Some(b'e') | Some(b'E')) {
        let exponent_start = end;
        end += 1;
        if matches!(bytes.get(end), Some(b'+') | Some(b'-')) {
            end += 1;
        }
        let digit_start = end;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end == digit_start {
            end = exponent_start;
        }
    }
    value[..end].parse::<f64>().ok()
}

pub fn parse_input_numeric_value(input_type: InputType, value: &str) -> Option<f64> {
    match input_type {
        InputType::Number | InputType::Range => parse_finite_number(value),
        InputType::Date => date_input_milliseconds(value),
        InputType::Time => time_input_milliseconds(value),
        InputType::DatetimeLocal => datetime_local_input_milliseconds(value),
        InputType::Month => month_input_number(value),
        InputType::Week => week_input_milliseconds(value),
        _ => None,
    }
}

pub fn input_number_to_value_string(input_type: InputType, value: f64) -> Option<String> {
    match input_type {
        InputType::Number | InputType::Range => Some(value.to_string()),
        InputType::Date => date_input_value_from_milliseconds(value),
        InputType::Time => time_input_value_from_milliseconds(value),
        InputType::DatetimeLocal => datetime_local_input_value_from_milliseconds(value),
        InputType::Month => month_input_value_from_number(value),
        InputType::Week => week_input_value_from_milliseconds(value),
        _ => None,
    }
}

pub fn input_step(input_type: InputType, step: Option<&str>) -> Option<f64> {
    let Some(step) = step else {
        return Some(default_input_step(input_type));
    };
    if step.eq_ignore_ascii_case("any") {
        return None;
    }
    Some(
        parse_finite_number(step)
            .filter(|value| *value > 0.0)
            .map(|value| value * input_step_scale(input_type))
            .unwrap_or_else(|| default_input_step(input_type)),
    )
}

fn default_input_step(input_type: InputType) -> f64 {
    match input_type {
        InputType::Date => MS_PER_DAY,
        InputType::Time | InputType::DatetimeLocal => 60.0 * MS_PER_SECOND,
        InputType::Week => MS_PER_WEEK,
        _ => 1.0,
    }
}

fn input_step_scale(input_type: InputType) -> f64 {
    match input_type {
        InputType::Date => MS_PER_DAY,
        InputType::Time | InputType::DatetimeLocal => MS_PER_SECOND,
        InputType::Week => MS_PER_WEEK,
        _ => 1.0,
    }
}

pub fn input_step_base(input_type: InputType, min: Option<&str>, value: Option<&str>) -> f64 {
    min.and_then(|value| parse_input_numeric_value(input_type, value))
        .or_else(|| value.and_then(|value| parse_input_numeric_value(input_type, value)))
        .unwrap_or_else(|| default_input_step_base(input_type))
}

fn default_input_step_base(input_type: InputType) -> f64 {
    match input_type {
        InputType::Week => WEEK_INPUT_STEP_BASE,
        _ => 0.0,
    }
}

pub fn number_aligns_to_step(value: f64, base: f64, step: f64) -> bool {
    let quotient = (value - base) / step;
    let distance = (quotient - quotient.round()).abs();
    distance <= 1e-7
}

pub fn input_range_underflow(
    input_type: InputType,
    value: f64,
    min: Option<&str>,
    max: Option<&str>,
) -> bool {
    if let Some((min, max)) = reversed_time_range(input_type, min, max) {
        return value > max && value < min;
    }
    min.and_then(|min| parse_input_numeric_value(input_type, min))
        .is_some_and(|min| value < min)
}

pub fn input_range_overflow(
    input_type: InputType,
    value: f64,
    min: Option<&str>,
    max: Option<&str>,
) -> bool {
    if let Some((min, max)) = reversed_time_range(input_type, min, max) {
        return value > max && value < min;
    }
    max.and_then(|max| parse_input_numeric_value(input_type, max))
        .is_some_and(|max| value > max)
}

fn reversed_time_range(
    input_type: InputType,
    min: Option<&str>,
    max: Option<&str>,
) -> Option<(f64, f64)> {
    if input_type != InputType::Time {
        return None;
    }
    let min = min.and_then(|min| parse_input_numeric_value(input_type, min))?;
    let max = max.and_then(|max| parse_input_numeric_value(input_type, max))?;
    (min > max).then_some((min, max))
}

pub fn number_step_mismatch(
    value: &str,
    step: Option<&str>,
    min: Option<&str>,
    value_attribute: Option<&str>,
) -> Option<bool> {
    let value_number = parse_finite_number(value)?;
    let step = normalized_number_step_decimal(step)?;
    let base = normalized_number_step_base_decimal(min, value_attribute).unwrap_or(DecimalNumber {
        coefficient: 0,
        scale: 0,
    });
    let value = parse_decimal_number(value).or_else(|| decimal_number_from_f64(value_number))?;
    Some(!decimal_number_aligns_to_step(value, base, step))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputStepDirection {
    Up,
    Down,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputStepOutcome {
    Set(String),
    NoChange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputStepError {
    Unsupported,
    NoAllowedStep,
}

#[derive(Clone, Copy, Debug)]
pub struct InputStepState<'a> {
    pub input_type: InputType,
    pub value: &'a str,
    pub min: Option<&'a str>,
    pub max: Option<&'a str>,
    pub step: Option<&'a str>,
    pub value_attribute: Option<&'a str>,
}

pub fn step_input_value(
    state: InputStepState<'_>,
    direction: InputStepDirection,
    n: f64,
) -> Result<InputStepOutcome, InputStepError> {
    if !state.input_type.supports_value_as_number() {
        return Err(InputStepError::Unsupported);
    }
    let Some(step) = input_step(state.input_type, state.step) else {
        return Err(InputStepError::NoAllowedStep);
    };
    let minimum = state
        .min
        .and_then(|value| parse_input_numeric_value(state.input_type, value));
    let maximum = state
        .max
        .and_then(|value| parse_input_numeric_value(state.input_type, value));
    if minimum.zip(maximum).is_some_and(|(min, max)| min > max) {
        return Ok(InputStepOutcome::NoChange);
    }

    let base = input_step_base(state.input_type, state.min, state.value_attribute);
    if let (Some(min), Some(max)) = (minimum, maximum) {
        let first = smallest_aligned_value_at_or_above(min, base, step);
        if first > max {
            return Ok(InputStepOutcome::NoChange);
        }
    }

    let parsed_value = parse_input_numeric_value(state.input_type, state.value);
    let had_value = parsed_value.is_some();
    let mut value = parsed_value.unwrap_or(0.0);
    let value_before_stepping = value;
    if number_aligns_to_step(value, base, step) {
        let delta = match direction {
            InputStepDirection::Up => step * n,
            InputStepDirection::Down => -(step * n),
        };
        value += delta;
    } else {
        value = nearest_aligned_value_in_direction(value, base, step, direction);
    }

    if let Some(minimum) = minimum.filter(|minimum| value < *minimum) {
        value = smallest_aligned_value_at_or_above(minimum, base, step);
    }
    if let Some(maximum) = maximum.filter(|maximum| value > *maximum) {
        value = largest_aligned_value_at_or_below(maximum, base, step);
    }
    if had_value
        && (matches!(direction, InputStepDirection::Down) && value > value_before_stepping
            || matches!(direction, InputStepDirection::Up) && value < value_before_stepping)
    {
        return Ok(InputStepOutcome::NoChange);
    }

    input_number_to_value_string(state.input_type, value)
        .map(InputStepOutcome::Set)
        .ok_or(InputStepError::Unsupported)
}

fn nearest_aligned_value_in_direction(
    value: f64,
    base: f64,
    step: f64,
    direction: InputStepDirection,
) -> f64 {
    let quotient = (value - base) / step;
    match direction {
        InputStepDirection::Up => {
            let candidate = base + quotient.ceil() * step;
            if candidate <= value {
                candidate + step
            } else {
                candidate
            }
        }
        InputStepDirection::Down => {
            let candidate = base + quotient.floor() * step;
            if candidate >= value {
                candidate - step
            } else {
                candidate
            }
        }
    }
}

fn smallest_aligned_value_at_or_above(value: f64, base: f64, step: f64) -> f64 {
    let candidate = base + ((value - base) / step).ceil() * step;
    if candidate < value {
        candidate + step
    } else {
        candidate
    }
}

fn largest_aligned_value_at_or_below(value: f64, base: f64, step: f64) -> f64 {
    let candidate = base + ((value - base) / step).floor() * step;
    if candidate > value {
        candidate - step
    } else {
        candidate
    }
}

fn normalized_number_step_decimal(step: Option<&str>) -> Option<DecimalNumber> {
    let Some(step) = step else {
        return Some(DecimalNumber {
            coefficient: 1,
            scale: 0,
        });
    };
    if step.eq_ignore_ascii_case("any") {
        return None;
    }
    let parsed = parse_finite_number(step)
        .filter(|value| *value > 0.0)
        .and_then(|_| parse_decimal_number(step));
    Some(parsed.unwrap_or(DecimalNumber {
        coefficient: 1,
        scale: 0,
    }))
}

fn normalized_number_step_base_decimal(
    min: Option<&str>,
    value_attribute: Option<&str>,
) -> Option<DecimalNumber> {
    min.filter(|value| parse_finite_number(value).is_some())
        .and_then(parse_decimal_number)
        .or_else(|| {
            value_attribute
                .filter(|value| parse_finite_number(value).is_some())
                .and_then(parse_decimal_number)
        })
}

fn parse_finite_number(value: &str) -> Option<f64> {
    let number = value.trim().parse::<f64>().ok()?;
    number.is_finite().then_some(number)
}

#[derive(Clone, Copy)]
struct DecimalNumber {
    coefficient: i128,
    scale: u32,
}

fn parse_decimal_number(value: &str) -> Option<DecimalNumber> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let (mantissa, exponent) = value
        .split_once(['e', 'E'])
        .map(|(mantissa, exponent)| Some((mantissa, exponent.parse::<i32>().ok()?)))
        .unwrap_or_else(|| Some((value, 0)))?;
    let (negative, mantissa) = match mantissa.as_bytes().first() {
        Some(b'+') => (false, &mantissa[1..]),
        Some(b'-') => (true, &mantissa[1..]),
        _ => (false, mantissa),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if integer.is_empty() && fraction.is_empty() {
        return None;
    }
    if !integer.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let digits = format!("{integer}{fraction}");
    let digits = digits.trim_start_matches('0');
    let mut coefficient = if digits.is_empty() {
        0
    } else {
        digits.parse::<i128>().ok()?
    };
    if negative {
        coefficient = -coefficient;
    }
    let scale = i32::try_from(fraction.len()).ok()?.checked_sub(exponent)?;
    if scale >= 0 {
        Some(DecimalNumber {
            coefficient,
            scale: scale as u32,
        })
    } else {
        let multiplier = checked_pow10((-scale) as u32)?;
        Some(DecimalNumber {
            coefficient: coefficient.checked_mul(multiplier)?,
            scale: 0,
        })
    }
}

fn decimal_number_from_f64(value: f64) -> Option<DecimalNumber> {
    parse_decimal_number(&value.to_string())
}

fn decimal_number_aligns_to_step(
    value: DecimalNumber,
    base: DecimalNumber,
    step: DecimalNumber,
) -> bool {
    if step.coefficient <= 0 {
        return false;
    }
    let scale = value.scale.max(base.scale).max(step.scale);
    let Some(value) = scale_decimal_number(value, scale) else {
        return false;
    };
    let Some(base) = scale_decimal_number(base, scale) else {
        return false;
    };
    let Some(step) = scale_decimal_number(step, scale) else {
        return false;
    };
    value
        .checked_sub(base)
        .is_some_and(|delta| delta.rem_euclid(step) == 0)
}

fn scale_decimal_number(value: DecimalNumber, scale: u32) -> Option<i128> {
    let multiplier = checked_pow10(scale.checked_sub(value.scale)?)?;
    value.coefficient.checked_mul(multiplier)
}

fn checked_pow10(power: u32) -> Option<i128> {
    let mut value = 1_i128;
    for _ in 0..power {
        value = value.checked_mul(10)?;
    }
    Some(value)
}
