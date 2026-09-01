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
    Px,
    Cm,
    Mm,
    In,
    Pt,
    Pc,
}

impl SvgLengthUnit {
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Number => "",
            Self::Percentage => "%",
            Self::Ems => "em",
            Self::Exs => "ex",
            Self::Px => "px",
            Self::Cm => "cm",
            Self::Mm => "mm",
            Self::In => "in",
            Self::Pt => "pt",
            Self::Pc => "pc",
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
    raw.trim()
        .to_ascii_lowercase()
        .parse::<SvgTypesLength>()
        .ok()
        .map(svg_length)
}

pub fn parse_length_list(raw: &str) -> Option<Vec<SvgLength>> {
    let normalized = raw.to_ascii_lowercase();
    LengthListParser::from(normalized.as_str())
        .map(|length| length.ok().map(svg_length))
        .collect()
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
