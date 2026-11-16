use indexmap::IndexMap;

use super::super::DomStringValue;

/// The ordered, case-sensitive attribute map associated with a processing
/// instruction. Data edits parse XML pseudo-attributes; attribute edits retain
/// this map while serializing data, including names that are not XML Names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessingInstructionAttributes {
    values: Option<Box<IndexMap<DomStringValue, DomStringValue>>>,
}

impl ProcessingInstructionAttributes {
    pub fn from_data(data: &DomStringValue) -> Self {
        let units = data.utf16_units();
        let Some(values) = parse_pseudo_attributes(&units) else {
            return Self::default();
        };
        Self {
            values: (!values.is_empty()).then(|| Box::new(values)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.values.as_ref().is_none_or(|values| values.is_empty())
    }

    pub fn keys(&self) -> impl Iterator<Item = &DomStringValue> {
        self.values.iter().flat_map(|values| values.keys())
    }

    pub fn get(&self, name: &DomStringValue) -> Option<&DomStringValue> {
        self.values.as_ref()?.get(name)
    }

    pub fn set(&mut self, name: DomStringValue, value: DomStringValue) {
        self.values
            .get_or_insert_with(Default::default)
            .insert(name, value);
    }

    pub fn remove(&mut self, name: &DomStringValue) -> bool {
        let Some(values) = self.values.as_mut() else {
            return false;
        };
        let removed = values.shift_remove(name).is_some();
        if values.is_empty() {
            self.values = None;
        }
        removed
    }

    pub fn serialize(&self) -> DomStringValue {
        let mut units = Vec::new();
        for (name, value) in self.values.iter().flat_map(|values| values.iter()) {
            if !units.is_empty() {
                units.push(u16::from(b' '));
            }
            name.append_utf16_units_to(&mut units);
            units.extend([u16::from(b'='), u16::from(b'"')]);
            for unit in value.utf16_units().iter().copied() {
                let escaped = match unit {
                    0x26 => Some("&amp;"),
                    0x3c => Some("&lt;"),
                    0x3e => Some("&gt;"),
                    0x22 => Some("&quot;"),
                    _ => None,
                };
                if let Some(escaped) = escaped {
                    units.extend(escaped.encode_utf16());
                } else {
                    units.push(unit);
                }
            }
            units.push(u16::from(b'"'));
        }
        DomStringValue::from_utf16(&units)
    }
}

// https://www.w3.org/TR/xml-stylesheet/#NT-PseudoAtts
// A malformed attribute or duplicate invalidates the entire parsing result.
fn parse_pseudo_attributes(input: &[u16]) -> Option<IndexMap<DomStringValue, DomStringValue>> {
    let mut values = IndexMap::new();
    let mut position = 0;
    skip_space(input, &mut position);
    while position < input.len() {
        let start = position;
        let (first, count) = scalar_at(input, position)?;
        if !xml_name_start(first) {
            return None;
        }
        position += count;
        while let Some((character, count)) = scalar_at(input, position) {
            if !xml_name_character(character) {
                break;
            }
            position += count;
        }
        let name = DomStringValue::from_utf16(&input[start..position]);
        if values.contains_key(&name) {
            return None;
        }
        skip_space(input, &mut position);
        if input.get(position) != Some(&u16::from(b'=')) {
            return None;
        }
        position += 1;
        skip_space(input, &mut position);
        let quote = *input.get(position)?;
        if quote != u16::from(b'\'') && quote != u16::from(b'"') {
            return None;
        }
        position += 1;
        let mut value = Vec::new();
        loop {
            let unit = *input.get(position)?;
            position += 1;
            if unit == quote {
                break;
            }
            match unit {
                0x3c => return None,
                0x26 => {
                    let start = position;
                    while *input.get(position)? != u16::from(b';') {
                        position += 1;
                    }
                    let scalar = character_reference(&input[start..position])?;
                    position += 1;
                    value.extend(scalar.encode_utf16(&mut [0; 2]).iter().copied());
                }
                _ => value.push(unit),
            }
        }
        values.insert(name, DomStringValue::from_utf16(&value));
        if position < input.len() && !xml_space(input[position]) {
            return None;
        }
        skip_space(input, &mut position);
    }
    Some(values)
}

fn xml_space(unit: u16) -> bool {
    matches!(unit, 0x9 | 0xa | 0xd | 0x20)
}

fn skip_space(input: &[u16], position: &mut usize) {
    while input.get(*position).is_some_and(|unit| xml_space(*unit)) {
        *position += 1;
    }
}

fn scalar_at(input: &[u16], position: usize) -> Option<(char, usize)> {
    let character = char::decode_utf16(input.get(position..)?.iter().copied())
        .next()?
        .ok()?;
    Some((character, character.len_utf16()))
}

fn xml_name_start(character: char) -> bool {
    matches!(character, ':' | '_' | 'A'..='Z' | 'a'..='z'
        | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}'
        | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}'
        | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}'
        | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
}

fn xml_name_character(character: char) -> bool {
    xml_name_start(character)
        || matches!(character, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}

fn character_reference(input: &[u16]) -> Option<char> {
    for (name, character) in [
        ("amp", '&'),
        ("lt", '<'),
        ("gt", '>'),
        ("quot", '"'),
        ("apos", '\''),
    ] {
        if input.iter().copied().eq(name.encode_utf16()) {
            return Some(character);
        }
    }
    if input.first() != Some(&u16::from(b'#')) {
        return None;
    }
    let (radix, digits) = if input.get(1) == Some(&u16::from(b'x')) {
        (16, &input[2..])
    } else {
        (10, &input[1..])
    };
    if digits.is_empty() {
        return None;
    }
    let mut scalar = 0u32;
    for unit in digits {
        let digit = char::from_u32(u32::from(*unit))?.to_digit(radix)?;
        scalar = scalar.checked_mul(radix)?.checked_add(digit)?;
    }
    if !matches!(scalar, 0x9 | 0xa | 0xd | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff) {
        return None;
    }
    char::from_u32(scalar)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudo_attributes_are_case_sensitive_and_decode_only_xml_references() {
        let data: DomStringValue =
            "A = '&lt;&#x1F95A;&#13;' a=\"&amp;&quot;&apos;\" ns:key='v'".into();
        let attributes = ProcessingInstructionAttributes::from_data(&data);
        assert_eq!(
            attributes
                .keys()
                .map(DomStringValue::as_str_lossy)
                .collect::<Vec<_>>(),
            ["A", "a", "ns:key"]
        );
        assert_eq!(attributes.get(&"A".into()).unwrap().as_str_lossy(), "<🥚\r");
        assert_eq!(attributes.get(&"a".into()).unwrap().as_str_lossy(), "&\"'");
        assert_eq!(
            ProcessingInstructionAttributes::from_data(&attributes.serialize()),
            attributes
        );
    }

    #[test]
    fn malformed_pseudo_attributes_clear_the_whole_map() {
        for data in [
            "a=b",
            "a='1' b",
            "a='1' a='2'",
            "a='&nbsp;'",
            "a='&#0;'",
            "a='&#xD800;'",
            "a='&#x110000;'",
            "a='&#X41;'",
            "a='<x'",
            "a='1'b='2'",
            "a='unterminated",
            "a='1'\u{c}b='2'",
        ] {
            assert!(
                ProcessingInstructionAttributes::from_data(&data.into()).is_empty(),
                "{data:?}"
            );
        }
    }

    #[test]
    fn attribute_serialization_preserves_order_values_and_utf16() {
        let mut attributes = ProcessingInstructionAttributes::default();
        attributes.set("A".into(), "first".into());
        attributes.set("a".into(), "<&>\"'".into());
        attributes.set("A".into(), "updated".into());
        attributes.set(
            "raw".into(),
            DomStringValue::from_utf16(&[0xd800, 0x61, 0xdc00]),
        );
        assert_eq!(
            attributes
                .keys()
                .map(DomStringValue::as_str_lossy)
                .collect::<Vec<_>>(),
            ["A", "a", "raw"]
        );
        assert_eq!(
            ProcessingInstructionAttributes::from_data(&attributes.serialize()),
            attributes
        );
        assert!(
            attributes
                .serialize()
                .as_str_lossy()
                .starts_with("A=\"updated\" a=\"&lt;&amp;&gt;&quot;'\" ")
        );
        assert!(!attributes.remove(&"missing".into()));
        assert!(attributes.remove(&"a".into()));
        assert_eq!(
            attributes
                .keys()
                .map(DomStringValue::as_str_lossy)
                .collect::<Vec<_>>(),
            ["A", "raw"]
        );
    }

    #[test]
    fn processing_instruction_attributes_reparse_equal_data_without_observers() {
        use crate::native::{DomHost, NativeDom};
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://pi-attributes.test/").unwrap(),
        ));
        assert!(!host.mutation_records_enabled());
        let value: DomStringValue = "odd\"key=\"value\"".into();
        let handle = host.create_processing_instruction("probe", value.clone());
        assert!(
            host.node(handle)
                .unwrap()
                .as_processing_instruction()
                .unwrap()
                .attributes()
                .is_empty()
        );
        let mut attributes = ProcessingInstructionAttributes::default();
        attributes.set("odd\"key".into(), "value".into());
        host.set_processing_instruction_attributes_effects(handle, attributes);
        assert_eq!(
            host.node(handle)
                .unwrap()
                .as_processing_instruction()
                .unwrap()
                .attributes()
                .get(&"odd\"key".into())
                .unwrap()
                .as_str_lossy(),
            "value"
        );
        host.set_character_data_value_effects(handle, value, false);
        assert!(
            host.node(handle)
                .unwrap()
                .as_processing_instruction()
                .unwrap()
                .attributes()
                .is_empty()
        );
    }
}
