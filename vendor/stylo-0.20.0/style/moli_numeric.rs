//! Moli-facing numeric value parsing and used-value resolution.
//!
//! This module deliberately keeps the CSS grammar and value type interaction in
//! Stylo while accepting the small set of runtime facts that Moli can
//! provide without a full layout engine.

use std::borrow::Cow;
use std::sync::Once;

use cssparser::{Parser, ParserInput, Token};
use style_traits::ParsingMode;

use crate::{
    context::QuirksMode,
    custom_properties::AttrTaint,
    parser::ParserContext,
    stylesheets::{CssRuleType, Namespaces, Origin, UrlExtraData},
    values::{
        generics::calc::{CalcUnits, MinMaxOp, ModRemOp, RoundingStrategy},
        specified::{
            calc::{CalcNode, CalcParseFlags, Leaf as CalcLeaf},
            length::LengthUnit,
            NoCalcLength, TreeCountingFunction as StyloTreeCountingFunction,
        },
    },
};

#[derive(Clone, Copy)]
pub enum UnitlessLength {
    Any,
    ZeroOnly,
}

#[derive(Clone, Copy)]
pub enum UnitlessAngle {
    Degrees,
    ZeroOnly,
}

#[derive(Clone, Copy)]
pub struct ContainerQueryLengthContext {
    pub width_px: f64,
    pub height_px: f64,
    pub inline_size_px: f64,
    pub block_size_px: f64,
}

impl ContainerQueryLengthContext {
    pub fn from_inline_size(width_px: f64) -> Self {
        Self {
            width_px,
            height_px: width_px,
            inline_size_px: width_px,
            block_size_px: width_px,
        }
    }
}

#[derive(Clone, Copy)]
pub enum CssNumericKind {
    Number,
    Percentage,
    Time,
    PxLength(UnitlessLength),
    LengthPercentage {
        basis: f64,
        unitless: UnitlessLength,
    },
    Angle(UnitlessAngle),
}

#[derive(Clone, Copy, Default)]
pub struct CssNumericContext {
    pub container_lengths: Option<ContainerQueryLengthContext>,
    pub font_size_px: Option<f64>,
    pub root_font_size_px: Option<f64>,
    pub line_height_px: Option<f64>,
    pub viewport_width_px: Option<f64>,
    pub viewport_height_px: Option<f64>,
    pub sibling_index: Option<f64>,
    pub sibling_count: Option<f64>,
}

impl CssNumericContext {
    pub fn supports_probe() -> Self {
        Self {
            container_lengths: Some(ContainerQueryLengthContext::from_inline_size(100.0)),
            font_size_px: Some(16.0),
            root_font_size_px: Some(16.0),
            line_height_px: Some(16.0),
            viewport_width_px: Some(100.0),
            viewport_height_px: Some(100.0),
            sibling_index: Some(1.0),
            sibling_count: Some(1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CssNumericValue {
    Number(f64),
    Percentage(f64),
    TimeSeconds(f64),
    PxLength(f64),
    AngleDegrees(f64),
}

impl CssNumericValue {
    pub fn number(self) -> Option<f64> {
        match self {
            Self::Number(value) if value.is_finite() => Some(value),
            _ => None,
        }
    }

    pub fn time_seconds(self) -> Option<f64> {
        match self {
            Self::TimeSeconds(value) if value.is_finite() => Some(value),
            _ => None,
        }
    }

    pub fn percentage(self) -> Option<f64> {
        match self {
            Self::Percentage(value) if value.is_finite() => Some(value),
            _ => None,
        }
    }

    pub fn px_length(self) -> Option<f64> {
        match self {
            Self::PxLength(value) if value.is_finite() => Some(value),
            _ => None,
        }
    }

    pub fn angle_degrees(self) -> Option<f64> {
        match self {
            Self::AngleDegrees(value) if value.is_finite() => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct MathOperand {
    value: f64,
    dimension_exponent: i8,
}

#[derive(Clone, Copy)]
enum StyloNumericKind {
    Number,
    Percentage,
    PxLength,
    LengthPercentage { basis: f64 },
    Angle,
    Time,
}

impl MathOperand {
    fn number(value: f64) -> Self {
        Self {
            value,
            dimension_exponent: 0,
        }
    }

    fn dimension(value: f64) -> Self {
        Self {
            value,
            dimension_exponent: 1,
        }
    }

    fn add(self, other: Self) -> Option<Self> {
        if self.dimension_exponent != other.dimension_exponent {
            return None;
        }
        Some(Self {
            value: self.value + other.value,
            dimension_exponent: self.dimension_exponent,
        })
    }

    fn negate(self) -> Self {
        Self {
            value: -self.value,
            dimension_exponent: self.dimension_exponent,
        }
    }

    fn multiply(self, other: Self) -> Option<Self> {
        Some(Self {
            value: self.value * other.value,
            dimension_exponent: self
                .dimension_exponent
                .checked_add(other.dimension_exponent)?,
        })
    }

    fn into_number(self) -> Option<f64> {
        (self.dimension_exponent == 0 && self.value.is_finite()).then_some(self.value)
    }

    fn into_dimension(self) -> Option<f64> {
        (self.dimension_exponent == 1 && self.value.is_finite()).then_some(self.value)
    }
}

pub fn parse_number(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    let value = trimmed.parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

pub fn parse_px_length(raw: &str, unitless: UnitlessLength) -> Option<f64> {
    resolve_css_numeric(
        raw,
        CssNumericKind::PxLength(unitless),
        CssNumericContext::default(),
    )?
    .px_length()
}

pub fn resolve_length_percentage(raw: &str, basis: f64, unitless: UnitlessLength) -> Option<f64> {
    resolve_css_numeric(
        raw,
        CssNumericKind::LengthPercentage { basis, unitless },
        CssNumericContext::default(),
    )?
    .px_length()
}

pub fn parse_angle_degrees(raw: &str, unitless: UnitlessAngle) -> Option<f64> {
    resolve_css_numeric(
        raw,
        CssNumericKind::Angle(unitless),
        CssNumericContext::default(),
    )?
    .angle_degrees()
}

pub fn resolve_time_seconds(
    raw: &str,
    container_lengths: Option<ContainerQueryLengthContext>,
) -> Option<f64> {
    resolve_css_numeric(
        raw,
        CssNumericKind::Time,
        CssNumericContext {
            container_lengths,
            ..CssNumericContext::default()
        },
    )?
    .time_seconds()
}

pub fn resolve_css_number(
    raw: &str,
    container_lengths: Option<ContainerQueryLengthContext>,
) -> Option<f64> {
    resolve_css_numeric(
        raw,
        CssNumericKind::Number,
        CssNumericContext {
            container_lengths,
            ..CssNumericContext::default()
        },
    )?
    .number()
}

pub fn resolve_css_numeric(
    raw: &str,
    kind: CssNumericKind,
    context: CssNumericContext,
) -> Option<CssNumericValue> {
    match kind {
        CssNumericKind::Number => resolve_stylo_number(raw, context).map(CssNumericValue::Number),
        CssNumericKind::Percentage => {
            resolve_stylo_percentage(raw, context).map(CssNumericValue::Percentage)
        },
        CssNumericKind::Time => {
            resolve_stylo_time_seconds(raw, context).map(CssNumericValue::TimeSeconds)
        },
        CssNumericKind::PxLength(unitless) => {
            resolve_stylo_px_length(raw, unitless, context).map(CssNumericValue::PxLength)
        },
        CssNumericKind::LengthPercentage { basis, unitless } => {
            resolve_stylo_length_percentage(raw, basis, unitless, context)
                .map(CssNumericValue::PxLength)
        },
        CssNumericKind::Angle(unitless) => {
            resolve_stylo_angle_degrees(raw, unitless, context).map(CssNumericValue::AngleDegrees)
        },
    }
}

pub fn css_time_value_is_supported(raw: &str) -> bool {
    css_numeric_value_is_supported(raw, CssNumericKind::Time)
}

pub fn css_number_value_is_supported(raw: &str) -> bool {
    css_numeric_value_is_supported(raw, CssNumericKind::Number)
}

pub fn css_numeric_value_is_supported(raw: &str, kind: CssNumericKind) -> bool {
    resolve_css_numeric(raw, kind, CssNumericContext::supports_probe()).is_some()
}

fn resolve_stylo_number(raw: &str, context: CssNumericContext) -> Option<f64> {
    let node = parse_stylo_numeric_node(raw, CalcUnits::ALL)?;
    eval_calc_node(&node, StyloNumericKind::Number, context)?.into_number()
}

fn resolve_stylo_percentage(raw: &str, context: CssNumericContext) -> Option<f64> {
    let node = parse_stylo_numeric_node(raw, CalcUnits::PERCENTAGE)?;
    eval_calc_node(&node, StyloNumericKind::Percentage, context)?.into_dimension()
}

fn resolve_stylo_time_seconds(raw: &str, context: CssNumericContext) -> Option<f64> {
    let node = parse_stylo_numeric_node(raw, CalcUnits::TIME)?;
    eval_calc_node(&node, StyloNumericKind::Time, context)?.into_dimension()
}

fn resolve_stylo_px_length(
    raw: &str,
    unitless: UnitlessLength,
    context: CssNumericContext,
) -> Option<f64> {
    parse_stylo_numeric_node(raw, CalcUnits::LENGTH)
        .and_then(|node| eval_calc_node(&node, StyloNumericKind::PxLength, context))
        .and_then(MathOperand::into_dimension)
        .or_else(|| resolve_unitless_length(raw, unitless))
}

fn resolve_stylo_length_percentage(
    raw: &str,
    basis: f64,
    unitless: UnitlessLength,
    context: CssNumericContext,
) -> Option<f64> {
    parse_stylo_numeric_node(raw, CalcUnits::LENGTH_PERCENTAGE)
        .and_then(|node| {
            eval_calc_node(&node, StyloNumericKind::LengthPercentage { basis }, context)
        })
        .and_then(MathOperand::into_dimension)
        .or_else(|| resolve_unitless_length(raw, unitless))
}

fn resolve_stylo_angle_degrees(
    raw: &str,
    unitless: UnitlessAngle,
    context: CssNumericContext,
) -> Option<f64> {
    parse_stylo_numeric_node(raw, CalcUnits::ANGLE)
        .and_then(|node| eval_calc_node(&node, StyloNumericKind::Angle, context))
        .and_then(MathOperand::into_dimension)
        .or_else(|| match unitless {
            UnitlessAngle::Degrees => parse_number(raw),
            UnitlessAngle::ZeroOnly => parse_number(raw).filter(|value| *value == 0.0),
        })
}

fn resolve_unitless_length(raw: &str, unitless: UnitlessLength) -> Option<f64> {
    let value = parse_number(raw)?;
    match unitless {
        UnitlessLength::Any => Some(value),
        UnitlessLength::ZeroOnly if value == 0.0 => Some(value),
        UnitlessLength::ZeroOnly => None,
    }
}

fn parse_stylo_numeric_node(raw: &str, units: CalcUnits) -> Option<CalcNode> {
    ensure_moli_numeric_prefs();
    // CalcNode::parse is Stylo's public numeric AST boundary. Wrapping plain
    // dimensions and percentages in calc() lets this adapter use that same
    // boundary without unpacking NumericUnion's private storage variants.
    let wrapped = format!("calc({})", raw.trim());
    with_stylo_numeric_context(ParsingMode::DEFAULT, |context| {
        let mut input = ParserInput::new(&wrapped);
        let mut parser = Parser::new(&mut input);
        let location = parser.current_source_location();
        let token = parser.next().ok()?.clone();
        let Token::Function(name) = token else {
            return None;
        };
        let function = CalcNode::math_function(context, &name, location).ok()?;
        let node =
            CalcNode::parse(context, &mut parser, function, CalcParseFlags::new(units)).ok()?;
        parser.expect_exhausted().ok()?;
        Some(node)
    })?
}

fn ensure_moli_numeric_prefs() {
    static ENABLE: Once = Once::new();
    ENABLE.call_once(|| {
        static_prefs::set_pref!("layout.css.tree-counting-functions.enabled", true);
    });
}

fn with_stylo_numeric_context<R>(
    parsing_mode: ParsingMode,
    f: impl FnOnce(&ParserContext) -> R,
) -> Option<R> {
    let url_data = UrlExtraData::from(url::Url::parse("about:blank").ok()?);
    let context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(CssRuleType::Style),
        parsing_mode,
        QuirksMode::NoQuirks,
        Cow::Owned(Namespaces::default()),
        None,
        None,
        AttrTaint::default(),
    );
    Some(f(&context))
}

fn eval_calc_node(
    node: &CalcNode,
    kind: StyloNumericKind,
    context: CssNumericContext,
) -> Option<MathOperand> {
    match node {
        CalcNode::Leaf(leaf) => eval_calc_leaf(leaf, kind, context),
        CalcNode::Negate(child) => Some(eval_calc_node(child, kind, context)?.negate()),
        CalcNode::Invert(child) => {
            let value = eval_calc_node(child, kind, context)?.into_number()?;
            (value != 0.0).then(|| MathOperand::number(1.0 / value))
        },
        CalcNode::Sum(children) => {
            let mut total: Option<MathOperand> = None;
            for child in children.iter() {
                let value = eval_calc_node(child, kind, context)?;
                total = Some(if let Some(total) = total {
                    total.add(value)?
                } else {
                    value
                });
            }
            total
        },
        CalcNode::Product(children) => {
            let mut product = MathOperand::number(1.0);
            for child in children.iter() {
                product = product.multiply(eval_calc_node(child, kind, context)?)?;
            }
            Some(product)
        },
        CalcNode::MinMax(children, op) => eval_calc_min_max(children.iter(), *op, kind, context),
        CalcNode::Clamp { min, center, max } => {
            let min = eval_calc_node(min, kind, context)?;
            let center = eval_calc_node(center, kind, context)?;
            let max = eval_calc_node(max, kind, context)?;
            if min.dimension_exponent != center.dimension_exponent
                || center.dimension_exponent != max.dimension_exponent
            {
                return None;
            }
            Some(MathOperand {
                value: min.value.max(center.value.min(max.value)),
                dimension_exponent: center.dimension_exponent,
            })
        },
        CalcNode::Round {
            strategy,
            value,
            step,
        } => eval_calc_round(*strategy, value, step, kind, context),
        CalcNode::ModRem {
            dividend,
            divisor,
            op,
        } => eval_calc_mod_rem(*op, dividend, divisor, kind, context),
        CalcNode::Hypot(children) => eval_calc_hypot(children.iter(), kind, context),
        CalcNode::Abs(child) => {
            let value = eval_calc_node(child, kind, context)?;
            Some(MathOperand {
                value: value.value.abs(),
                dimension_exponent: value.dimension_exponent,
            })
        },
        CalcNode::Sign(child) => {
            let value = eval_calc_node(child, kind, context)?;
            Some(MathOperand::number(value.value.signum()))
        },
        // These 0.20 nodes need unit-specific result contracts beyond the
        // length/angle/number used-value surface exposed by this adapter.
        CalcNode::Sin(_)
        | CalcNode::Cos(_)
        | CalcNode::Tan(_)
        | CalcNode::Asin(_)
        | CalcNode::Acos(_)
        | CalcNode::Atan(_)
        | CalcNode::Atan2(_, _)
        | CalcNode::Pow(_, _)
        | CalcNode::Sqrt(_)
        | CalcNode::Log(_, _)
        | CalcNode::Exp(_)
        | CalcNode::Progress { .. } => None,
        CalcNode::Anchor(_) | CalcNode::AnchorSize(_) => None,
    }
}

fn eval_calc_min_max<'a>(
    mut children: impl Iterator<Item = &'a CalcNode>,
    op: MinMaxOp,
    kind: StyloNumericKind,
    context: CssNumericContext,
) -> Option<MathOperand> {
    let mut result = eval_calc_node(children.next()?, kind, context)?;
    for child in children {
        let value = eval_calc_node(child, kind, context)?;
        if value.dimension_exponent != result.dimension_exponent {
            return None;
        }
        result.value = match op {
            MinMaxOp::Min => result.value.min(value.value),
            MinMaxOp::Max => result.value.max(value.value),
        };
    }
    Some(result)
}

fn eval_calc_round(
    strategy: RoundingStrategy,
    value: &CalcNode,
    step: &CalcNode,
    kind: StyloNumericKind,
    context: CssNumericContext,
) -> Option<MathOperand> {
    let value = eval_calc_node(value, kind, context)?;
    let step = eval_calc_node(step, kind, context)?;
    if value.dimension_exponent != step.dimension_exponent || step.value == 0.0 {
        return None;
    }
    let quotient = value.value / step.value;
    let rounded = match strategy {
        RoundingStrategy::Nearest => quotient.round(),
        RoundingStrategy::Up => quotient.ceil(),
        RoundingStrategy::Down => quotient.floor(),
        RoundingStrategy::ToZero => quotient.trunc(),
    };
    Some(MathOperand {
        value: rounded * step.value,
        dimension_exponent: value.dimension_exponent,
    })
}

fn eval_calc_mod_rem(
    op: ModRemOp,
    dividend: &CalcNode,
    divisor: &CalcNode,
    kind: StyloNumericKind,
    context: CssNumericContext,
) -> Option<MathOperand> {
    let dividend = eval_calc_node(dividend, kind, context)?;
    let divisor = eval_calc_node(divisor, kind, context)?;
    if dividend.dimension_exponent != divisor.dimension_exponent || divisor.value == 0.0 {
        return None;
    }
    let value = match op {
        ModRemOp::Mod => dividend.value - divisor.value * (dividend.value / divisor.value).floor(),
        ModRemOp::Rem => dividend.value - divisor.value * (dividend.value / divisor.value).trunc(),
    };
    Some(MathOperand {
        value,
        dimension_exponent: dividend.dimension_exponent,
    })
}

fn eval_calc_hypot<'a>(
    mut children: impl Iterator<Item = &'a CalcNode>,
    kind: StyloNumericKind,
    context: CssNumericContext,
) -> Option<MathOperand> {
    let first = eval_calc_node(children.next()?, kind, context)?;
    let mut sum = first.value * first.value;
    for child in children {
        let value = eval_calc_node(child, kind, context)?;
        if value.dimension_exponent != first.dimension_exponent {
            return None;
        }
        sum += value.value * value.value;
    }
    Some(MathOperand {
        value: sum.sqrt(),
        dimension_exponent: first.dimension_exponent,
    })
}

fn eval_calc_leaf(
    leaf: &CalcLeaf,
    kind: StyloNumericKind,
    context: CssNumericContext,
) -> Option<MathOperand> {
    match leaf {
        CalcLeaf::Number(value) => Some(MathOperand::number(f64::from(value.get()))),
        CalcLeaf::TreeCountingFunction(function) => match function {
            StyloTreeCountingFunction::SiblingIndex => context.sibling_index,
            StyloTreeCountingFunction::SiblingCount => context.sibling_count,
        }
        .map(MathOperand::number),
        CalcLeaf::Percentage(value) => match kind {
            StyloNumericKind::Percentage => {
                Some(MathOperand::dimension(f64::from(value.get()) * 100.0))
            },
            StyloNumericKind::LengthPercentage { basis } => {
                Some(MathOperand::dimension(basis * f64::from(value.get())))
            },
            _ => None,
        },
        CalcLeaf::Length(length) => {
            resolve_no_calc_length(length, context).map(MathOperand::dimension)
        },
        CalcLeaf::Angle(angle) => Some(MathOperand::dimension(f64::from(angle.degrees()))),
        CalcLeaf::Time(time) => Some(MathOperand::dimension(f64::from(time.seconds()))),
        CalcLeaf::Resolution(_) | CalcLeaf::ColorComponent(_) => None,
    }
}

fn resolve_no_calc_length(length: &NoCalcLength, context: CssNumericContext) -> Option<f64> {
    let value = f64::from(length.unitless_value());
    match length.length_unit() {
        LengthUnit::Px
        | LengthUnit::In
        | LengthUnit::Cm
        | LengthUnit::Mm
        | LengthUnit::Q
        | LengthUnit::Pt
        | LengthUnit::Pc => length.to_px_if_absolute().map(f64::from),
        LengthUnit::Em => context.font_size_px.map(|basis| basis * value),
        LengthUnit::Rem => context.root_font_size_px.map(|basis| basis * value),
        LengthUnit::Lh | LengthUnit::Rlh => context.line_height_px.map(|basis| basis * value),
        LengthUnit::Ex | LengthUnit::Ch => context.font_size_px.map(|basis| basis * 0.5 * value),
        LengthUnit::Rex | LengthUnit::Rch => {
            context.root_font_size_px.map(|basis| basis * 0.5 * value)
        },
        LengthUnit::Ic => context.font_size_px.map(|basis| basis * value),
        LengthUnit::Ric => context.root_font_size_px.map(|basis| basis * value),
        LengthUnit::Cap => context.font_size_px.map(|basis| basis * 0.7 * value),
        LengthUnit::Rcap => context.root_font_size_px.map(|basis| basis * 0.7 * value),
        LengthUnit::Vw
        | LengthUnit::Svw
        | LengthUnit::Lvw
        | LengthUnit::Dvw
        | LengthUnit::Vi
        | LengthUnit::Svi
        | LengthUnit::Lvi
        | LengthUnit::Dvi => context.viewport_width_px.map(|basis| basis * value / 100.0),
        LengthUnit::Vh
        | LengthUnit::Svh
        | LengthUnit::Lvh
        | LengthUnit::Dvh
        | LengthUnit::Vb
        | LengthUnit::Svb
        | LengthUnit::Lvb
        | LengthUnit::Dvb => context
            .viewport_height_px
            .map(|basis| basis * value / 100.0),
        LengthUnit::Vmin | LengthUnit::Svmin | LengthUnit::Lvmin | LengthUnit::Dvmin => {
            Some(context.viewport_width_px?.min(context.viewport_height_px?) * value / 100.0)
        },
        LengthUnit::Vmax | LengthUnit::Svmax | LengthUnit::Lvmax | LengthUnit::Dvmax => {
            Some(context.viewport_width_px?.max(context.viewport_height_px?) * value / 100.0)
        },
        LengthUnit::Cqw => context
            .container_lengths
            .map(|lengths| lengths.width_px * value / 100.0),
        LengthUnit::Cqh => context
            .container_lengths
            .map(|lengths| lengths.height_px * value / 100.0),
        LengthUnit::Cqi => context
            .container_lengths
            .map(|lengths| lengths.inline_size_px * value / 100.0),
        LengthUnit::Cqb => context
            .container_lengths
            .map(|lengths| lengths.block_size_px * value / 100.0),
        LengthUnit::Cqmin => context
            .container_lengths
            .map(|lengths| lengths.inline_size_px.min(lengths.block_size_px) * value / 100.0),
        LengthUnit::Cqmax => context
            .container_lengths
            .map(|lengths| lengths.inline_size_px.max(lengths.block_size_px) * value / 100.0),
        LengthUnit::ServoCharacterWidth => None,
    }
}

pub fn starts_with_supported_math_function(raw: &str) -> bool {
    let mut input = ParserInput::new(raw.trim());
    let mut parser = Parser::new(&mut input);
    with_stylo_numeric_context(ParsingMode::DEFAULT, |context| {
        let location = parser.current_source_location();
        matches!(
            parser.next(),
            Ok(Token::Function(name))
                if CalcNode::math_function(context, name, location).is_ok()
        )
    })
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        css_number_value_is_supported, css_time_value_is_supported, parse_angle_degrees,
        parse_number, parse_px_length, resolve_css_number, resolve_css_numeric,
        resolve_length_percentage, resolve_time_seconds, starts_with_supported_math_function,
        ContainerQueryLengthContext, CssNumericContext, CssNumericKind, CssNumericValue,
        UnitlessAngle, UnitlessLength,
    };

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "expected {actual} to be close to {expected}"
        );
    }

    #[test]
    fn css_math_lengths_and_angles_parse_absolute_values() {
        assert_eq!(parse_number(" 12.5 "), Some(12.5));
        assert_eq!(parse_number("Infinity"), None);
        assert_close(parse_px_length("10px", UnitlessLength::Any).unwrap(), 10.0);
        assert_close(parse_px_length("10", UnitlessLength::Any).unwrap(), 10.0);
        assert_eq!(parse_px_length("10", UnitlessLength::ZeroOnly), None);
        assert_close(
            parse_px_length("calc(10px + 2px)", UnitlessLength::Any).unwrap(),
            12.0,
        );
        assert_close(
            parse_px_length("calc(2px * 3)", UnitlessLength::Any).unwrap(),
            6.0,
        );
        assert_close(
            parse_px_length("clamp(1px, max(2px, 3px), 4px)", UnitlessLength::Any).unwrap(),
            3.0,
        );
        assert_close(
            parse_angle_degrees("0.5turn", UnitlessAngle::Degrees).unwrap(),
            180.0,
        );
        assert_close(
            parse_angle_degrees("calc(90deg + 1rad)", UnitlessAngle::Degrees).unwrap(),
            90.0 + 180.0 / std::f64::consts::PI,
        );
    }

    #[test]
    fn css_math_uses_stylo_parser_for_extended_functions_and_unit_algebra() {
        assert!(starts_with_supported_math_function("calc(1px + 2px)"));
        assert!(starts_with_supported_math_function("round(up, 2.2px, 1px)"));
        assert_close(
            parse_px_length("abs(-2px)", UnitlessLength::ZeroOnly).unwrap(),
            2.0,
        );
        assert_close(
            parse_px_length("round(up, 2.2px, 1px)", UnitlessLength::ZeroOnly).unwrap(),
            3.0,
        );
        assert_eq!(resolve_css_number("calc(5px / 1px)", None), None);
        assert_eq!(
            parse_px_length("calc(5px / 1px)", UnitlessLength::ZeroOnly),
            None
        );
    }

    #[test]
    fn css_math_resolves_length_percentages_against_basis() {
        assert_close(
            resolve_length_percentage("25%", 200.0, UnitlessLength::ZeroOnly).unwrap(),
            50.0,
        );
        assert_close(
            resolve_length_percentage("calc(25% - 2px)", 200.0, UnitlessLength::ZeroOnly).unwrap(),
            48.0,
        );
        assert_close(
            resolve_length_percentage("max(10%, 3px)", 20.0, UnitlessLength::ZeroOnly).unwrap(),
            3.0,
        );
        assert_close(
            resolve_length_percentage(
                "max(10px + (2 * (10px + min(10%, 30px))), 5% + 80px)",
                100.0,
                UnitlessLength::ZeroOnly,
            )
            .unwrap(),
            85.0,
        );
    }

    #[test]
    fn css_math_resolves_percentages_without_accepting_lengths() {
        assert_close(
            resolve_css_numeric(
                "calc(min(50%, 60%))",
                CssNumericKind::Percentage,
                CssNumericContext::supports_probe(),
            )
            .unwrap()
            .percentage()
            .unwrap(),
            50.0,
        );
        assert!(resolve_css_numeric(
            "calc(50px - 50%)",
            CssNumericKind::Percentage,
            CssNumericContext::supports_probe(),
        )
        .is_none());
    }

    #[test]
    fn css_math_resolves_sign_with_container_units_for_animation_values() {
        let container = ContainerQueryLengthContext::from_inline_size(100.0);
        assert!(css_time_value_is_supported(
            "calc(10s + (sign(2cqw - 10px) * 5s))"
        ));
        assert!(css_number_value_is_supported(
            "calc(10 + (sign(2cqw - 10px) * 5))"
        ));
        assert_close(
            resolve_time_seconds("calc(10s + (sign(2cqw - 10px) * 5s))", Some(container)).unwrap(),
            5.0,
        );
        assert_close(
            resolve_css_number("calc(10 + (sign(2cqw - 10px) * 5))", Some(container)).unwrap(),
            5.0,
        );
    }

    #[test]
    fn css_math_resolves_shared_numeric_context_units() {
        static_prefs::set_pref!("layout.css.tree-counting-functions.enabled", true);

        let context = CssNumericContext {
            container_lengths: Some(ContainerQueryLengthContext::from_inline_size(200.0)),
            font_size_px: Some(16.0),
            root_font_size_px: Some(30.0),
            line_height_px: Some(20.0),
            viewport_width_px: Some(1920.0),
            viewport_height_px: Some(1080.0),
            ..CssNumericContext::default()
        };
        assert_close(
            resolve_css_numeric(
                "10%",
                CssNumericKind::LengthPercentage {
                    basis: 520.0,
                    unitless: UnitlessLength::ZeroOnly,
                },
                context,
            )
            .unwrap()
            .px_length()
            .unwrap(),
            52.0,
        );
        assert_close(
            resolve_css_numeric(
                "2em",
                CssNumericKind::LengthPercentage {
                    basis: 520.0,
                    unitless: UnitlessLength::ZeroOnly,
                },
                context,
            )
            .unwrap()
            .px_length()
            .unwrap(),
            32.0,
        );
        assert!(
            resolve_css_numeric(
                "calc(10% + 2px)",
                CssNumericKind::LengthPercentage {
                    basis: 520.0,
                    unitless: UnitlessLength::ZeroOnly,
                },
                context,
            )
            .is_some(),
            "percentage plus px should resolve"
        );
        assert!(
            resolve_css_numeric(
                "calc(10px + 2em)",
                CssNumericKind::LengthPercentage {
                    basis: 520.0,
                    unitless: UnitlessLength::ZeroOnly,
                },
                context,
            )
            .is_some(),
            "px plus em should resolve"
        );
        assert_close(
            resolve_css_numeric(
                "calc(10% + 2em)",
                CssNumericKind::LengthPercentage {
                    basis: 520.0,
                    unitless: UnitlessLength::ZeroOnly,
                },
                context,
            )
            .unwrap()
            .px_length()
            .unwrap(),
            84.0,
        );
        assert_close(
            resolve_css_numeric(
                "calc(8lh + 7px)",
                CssNumericKind::LengthPercentage {
                    basis: 100.0,
                    unitless: UnitlessLength::ZeroOnly,
                },
                context,
            )
            .unwrap()
            .px_length()
            .unwrap(),
            167.0,
        );
        let resolve_width = |value| {
            resolve_css_numeric(
                value,
                CssNumericKind::LengthPercentage {
                    basis: 520.0,
                    unitless: UnitlessLength::ZeroOnly,
                },
                context,
            )
            .and_then(CssNumericValue::px_length)
        };
        assert_close(resolve_width("calc(5px * 10)").unwrap(), 50.0);
        assert_close(resolve_width("calc(20% * 0.5)").unwrap(), 52.0);
        assert_close(resolve_width("calc(4px * 4)").unwrap(), 16.0);
        assert_close(resolve_width("calc(400px / 4)").unwrap(), 100.0);
        assert_close(resolve_width("calc((20% + 1em) * 0.5)").unwrap(), 60.0);
        assert_close(resolve_width("calc(100px / 1 / 1)").unwrap(), 100.0);
        assert_eq!(resolve_width("calc(5px * 10lh / 1px)"), None);
        assert_eq!(resolve_width("calc(20% * 0.5em / 1px)"), None);
        assert_eq!(resolve_width("calc(400px / 4lh * 1px)"), None);
        assert_eq!(resolve_width("calc(20% / 0.5em * 1px)"), None);
        assert_eq!(resolve_width("calc(52px * 1px / 10%)"), None);
        assert_eq!(resolve_width("calc(100px * 1px / 1px / 1)"), None);
        assert_close(
            resolve_css_numeric(
                "calc(10 + sign(1em - 1000px))",
                CssNumericKind::Number,
                context,
            )
            .unwrap()
            .number()
            .unwrap(),
            9.0,
        );
        assert_close(
            resolve_css_numeric("calc(10 + sign(1 - 2))", CssNumericKind::Number, context)
                .unwrap()
                .number()
                .unwrap(),
            9.0,
        );
        assert_close(
            resolve_css_numeric(
                "calc(10 + sign(30deg - 40deg))",
                CssNumericKind::Number,
                context,
            )
            .unwrap()
            .number()
            .unwrap(),
            9.0,
        );
        assert_close(
            resolve_css_numeric(
                "calc(2 * sibling-index())",
                CssNumericKind::Number,
                CssNumericContext {
                    sibling_index: Some(3.0),
                    ..context
                },
            )
            .unwrap()
            .number()
            .unwrap(),
            6.0,
        );
    }
}
