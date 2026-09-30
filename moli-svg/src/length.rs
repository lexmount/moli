use svgtypes::{
    Length as SvgTypesLength, LengthListParser, LengthUnit as SvgTypesLengthUnit,
    Number as SvgTypesNumber, NumberListParser,
};

use crate::matrix::serialize_number;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvgLengthUnit {
    Number,
    Percentage,
    Ems,
    Exs,
    Ch,
    Rem,
    Lh,
    Rlh,
    Cap,
    Ic,
    Px,
    Cm,
    Mm,
    Q,
    In,
    Pt,
    Pc,
    Vw,
    Vh,
    Vmin,
    Vmax,
}

impl SvgLengthUnit {
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Number => "",
            Self::Percentage => "%",
            Self::Ems => "em",
            Self::Exs => "ex",
            Self::Ch => "ch",
            Self::Rem => "rem",
            Self::Lh => "lh",
            Self::Rlh => "rlh",
            Self::Cap => "cap",
            Self::Ic => "ic",
            Self::Px => "px",
            Self::Cm => "cm",
            Self::Mm => "mm",
            Self::Q => "q",
            Self::In => "in",
            Self::Pt => "pt",
            Self::Pc => "pc",
            Self::Vw => "vw",
            Self::Vh => "vh",
            Self::Vmin => "vmin",
            Self::Vmax => "vmax",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgLength {
    pub value: f64,
    pub unit: SvgLengthUnit,
}

impl SvgLength {
    pub fn new(value: f64, unit: SvgLengthUnit) -> Self {
        Self { value, unit }
    }

    pub fn serialize(self) -> String {
        let mut serialized = serialize_number(self.value);
        serialized.push_str(self.unit.suffix());
        serialized
    }
}

pub fn parse_length(raw: &str) -> Option<SvgLength> {
    let normalized = raw.trim().to_ascii_lowercase();
    normalized
        .parse::<SvgTypesLength>()
        .ok()
        .map(svg_length)
        .or_else(|| parse_extended_length(&normalized))
}

pub fn parse_length_list(raw: &str) -> Option<Vec<SvgLength>> {
    let normalized = raw.to_ascii_lowercase();
    let parsed = LengthListParser::from(normalized.as_str())
        .map(|length| length.ok().map(svg_length))
        .collect::<Option<Vec<_>>>();
    parsed.or_else(|| parse_extended_length_list(&normalized))
}

pub fn parse_number(raw: &str) -> Option<f64> {
    raw.trim()
        .parse::<SvgTypesNumber>()
        .ok()
        .map(|number| number.0)
}

pub fn parse_number_list(raw: &str) -> Option<Vec<f64>> {
    NumberListParser::from(raw)
        .map(|number| number.ok())
        .collect()
}

pub fn parse_point_list(raw: &str) -> Option<Vec<(f64, f64)>> {
    let values = parse_number_list(raw).or_else(|| {
        let raw = raw.trim_end();
        parse_number_list(raw.strip_suffix(',')?)
    })?;
    Some(
        values
            .chunks_exact(2)
            .map(|point| (point[0], point[1]))
            .collect(),
    )
}

fn svg_length(length: SvgTypesLength) -> SvgLength {
    SvgLength {
        value: length.number,
        unit: match length.unit {
            SvgTypesLengthUnit::None => SvgLengthUnit::Number,
            SvgTypesLengthUnit::Em => SvgLengthUnit::Ems,
            SvgTypesLengthUnit::Ex => SvgLengthUnit::Exs,
            SvgTypesLengthUnit::Px => SvgLengthUnit::Px,
            SvgTypesLengthUnit::In => SvgLengthUnit::In,
            SvgTypesLengthUnit::Cm => SvgLengthUnit::Cm,
            SvgTypesLengthUnit::Mm => SvgLengthUnit::Mm,
            SvgTypesLengthUnit::Pt => SvgLengthUnit::Pt,
            SvgTypesLengthUnit::Pc => SvgLengthUnit::Pc,
            SvgTypesLengthUnit::Percent => SvgLengthUnit::Percentage,
        },
    }
}

fn parse_extended_length(raw: &str) -> Option<SvgLength> {
    let number_len = number_prefix_len(raw)?;
    let value = raw[..number_len].parse::<f64>().ok()?;
    if !value.is_finite() {
        return None;
    }
    let unit = match &raw[number_len..] {
        "ch" => SvgLengthUnit::Ch,
        "rem" => SvgLengthUnit::Rem,
        "lh" => SvgLengthUnit::Lh,
        "rlh" => SvgLengthUnit::Rlh,
        "cap" => SvgLengthUnit::Cap,
        "ic" => SvgLengthUnit::Ic,
        "q" => SvgLengthUnit::Q,
        "vw" => SvgLengthUnit::Vw,
        "vh" => SvgLengthUnit::Vh,
        "vmin" => SvgLengthUnit::Vmin,
        "vmax" => SvgLengthUnit::Vmax,
        _ => return None,
    };
    Some(SvgLength::new(value, unit))
}

fn parse_extended_length_list(raw: &str) -> Option<Vec<SvgLength>> {
    let bytes = raw.as_bytes();
    let mut index = 0;
    let mut lengths = Vec::new();

    skip_ascii_whitespace(bytes, &mut index);
    if index == bytes.len() {
        return Some(lengths);
    }

    loop {
        let number_len = number_prefix_len(&raw[index..])?;
        let number_end = index + number_len;
        let mut token_end = number_end;
        if matches!(bytes.get(token_end), Some(b'%')) {
            token_end += 1;
        } else {
            while matches!(bytes.get(token_end), Some(b'a'..=b'z')) {
                token_end += 1;
            }
        }
        lengths.push(parse_length(&raw[index..token_end])?);
        index = token_end;
        if index == bytes.len() {
            return Some(lengths);
        }

        let separator_start = index;
        skip_ascii_whitespace(bytes, &mut index);
        if index == bytes.len() {
            return Some(lengths);
        }
        if matches!(bytes.get(index), Some(b',')) {
            index += 1;
            skip_ascii_whitespace(bytes, &mut index);
            if index == bytes.len() || matches!(bytes.get(index), Some(b',')) {
                return None;
            }
        } else if index == separator_start && !matches!(bytes.get(index), Some(b'+' | b'-')) {
            return None;
        }
    }
}

fn skip_ascii_whitespace(bytes: &[u8], index: &mut usize) {
    while matches!(
        bytes.get(*index),
        Some(b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
    ) {
        *index += 1;
    }
}

fn number_prefix_len(raw: &str) -> Option<usize> {
    let bytes = raw.as_bytes();
    let mut index = 0;
    if matches!(bytes.get(index), Some(b'+' | b'-')) {
        index += 1;
    }

    let integer_start = index;
    while matches!(bytes.get(index), Some(b'0'..=b'9')) {
        index += 1;
    }
    let integer_digits = index - integer_start;

    let mut fraction_digits = 0;
    if matches!(bytes.get(index), Some(b'.')) {
        index += 1;
        let fraction_start = index;
        while matches!(bytes.get(index), Some(b'0'..=b'9')) {
            index += 1;
        }
        fraction_digits = index - fraction_start;
    }
    if integer_digits == 0 && fraction_digits == 0 {
        return None;
    }

    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while matches!(bytes.get(index), Some(b'0'..=b'9')) {
            index += 1;
        }
        if exponent_start == index {
            return None;
        }
    }
    Some(index)
}
