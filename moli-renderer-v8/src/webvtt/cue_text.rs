//! WebVTT cue text parsing and the data needed by DOM construction (sections 6.4–6.5).
//!
//! Keep script-created DOMStrings as UTF-16. File decoding and newline/NUL
//! normalization belong to the file parser, not to this tokenizer.

use markup5ever::data::{C1_REPLACEMENTS, NAMED_ENTITIES};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CueTag {
    Class,
    Italic,
    Bold,
    Underline,
    Ruby,
    RubyText,
    Voice,
    Language,
}

impl CueTag {
    fn parse(name: &[u16]) -> Option<Self> {
        Some(match String::from_utf16(name).ok()?.as_str() {
            "c" => Self::Class,
            "i" => Self::Italic,
            "b" => Self::Bold,
            "u" => Self::Underline,
            "ruby" => Self::Ruby,
            "rt" => Self::RubyText,
            "v" => Self::Voice,
            "lang" => Self::Language,
            _ => return None,
        })
    }

    pub(crate) fn element_name(self) -> &'static str {
        match self {
            Self::Class | Self::Voice | Self::Language => "span",
            Self::Italic => "i",
            Self::Bold => "b",
            Self::Underline => "u",
            Self::Ruby => "ruby",
            Self::RubyText => "rt",
        }
    }

    pub(crate) fn annotation_attribute(self) -> Option<&'static str> {
        match self {
            Self::Voice => Some("title"),
            Self::Language => Some("lang"),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum CueNodeKind {
    Text(Vec<u16>),
    Element {
        tag: CueTag,
        classes: Vec<u16>,
        annotation: Vec<u16>,
    },
    Timestamp(String),
}

#[derive(Debug, PartialEq)]
pub(crate) struct CueNode {
    /// Index of an earlier node, or None for the root list/DocumentFragment.
    pub(crate) parent: Option<usize>,
    pub(crate) kind: CueNodeKind,
}

/// A flat tree avoids recursion both when parsing deeply nested cue text and
/// when constructing its DOM. Only language spans expose a language attribute;
/// inherited applicable language on the other WebVTT objects is not DOM state.
pub(crate) fn parse_cue_text(source: &[u16]) -> Vec<CueNode> {
    let mut nodes = Vec::new();
    let mut stack: Vec<(usize, CueTag)> = Vec::new();
    let mut tokenizer = CueTextTokenizer { source, cursor: 0 };
    while let Some(token) = tokenizer.next_token() {
        let kind = match token {
            CueToken::Text(text) => CueNodeKind::Text(text),
            CueToken::Start {
                tag: Some(tag),
                classes,
                annotation,
            } => {
                if tag == CueTag::RubyText
                    && !stack.last().is_some_and(|(_, tag)| *tag == CueTag::Ruby)
                {
                    continue;
                }
                let parent = stack.last().map(|(index, _)| *index);
                stack.push((nodes.len(), tag));
                nodes.push(CueNode {
                    parent,
                    kind: CueNodeKind::Element {
                        tag,
                        classes,
                        annotation,
                    },
                });
                continue;
            }
            CueToken::End(Some(tag)) => {
                if let Some((_, current)) = stack.last() {
                    if *current == tag {
                        stack.pop();
                    } else if tag == CueTag::Ruby && *current == CueTag::RubyText {
                        stack.pop();
                        stack.pop();
                    }
                }
                continue;
            }
            CueToken::Timestamp(raw) => {
                let Some(timestamp) = canonical_timestamp(&raw) else {
                    continue;
                };
                CueNodeKind::Timestamp(timestamp)
            }
            CueToken::Start { tag: None, .. } | CueToken::End(None) => continue,
        };
        nodes.push(CueNode {
            parent: stack.last().map(|(index, _)| *index),
            kind,
        });
    }
    nodes
}

enum CueToken {
    Text(Vec<u16>),
    Start {
        tag: Option<CueTag>,
        classes: Vec<u16>,
        annotation: Vec<u16>,
    },
    End(Option<CueTag>),
    Timestamp(Vec<u16>),
}

struct CueTextTokenizer<'a> {
    source: &'a [u16],
    cursor: usize,
}

impl CueTextTokenizer<'_> {
    fn next_token(&mut self) -> Option<CueToken> {
        let &first = self.source.get(self.cursor)?;
        if first != u16::from(b'<') {
            let mut text = Vec::new();
            while let Some(&unit) = self.source.get(self.cursor) {
                if unit == u16::from(b'<') {
                    break;
                }
                self.cursor += 1;
                if unit == u16::from(b'&') {
                    consume_reference(self.source, &mut self.cursor, &mut text);
                } else {
                    text.push(unit);
                }
            }
            return Some(CueToken::Text(text));
        }
        self.cursor += 1;
        match self.source.get(self.cursor).copied() {
            Some(0x2f) => {
                self.cursor += 1;
                let name = self.until_tag_end();
                Some(CueToken::End(CueTag::parse(name)))
            }
            Some(0x30..=0x39) => Some(CueToken::Timestamp(self.until_tag_end().to_vec())),
            _ => Some(self.start_tag()),
        }
    }

    fn until_tag_end(&mut self) -> &[u16] {
        let start = self.cursor;
        while self
            .source
            .get(self.cursor)
            .is_some_and(|&unit| unit != u16::from(b'>'))
        {
            self.cursor += 1;
        }
        let end = self.cursor;
        self.consume_tag_end();
        &self.source[start..end]
    }

    fn consume_tag_end(&mut self) {
        if self.source.get(self.cursor) == Some(&u16::from(b'>')) {
            self.cursor += 1;
        }
    }

    fn start_tag(&mut self) -> CueToken {
        let start = self.cursor;
        while self
            .source
            .get(self.cursor)
            .is_some_and(|&unit| !tag_whitespace(unit) && !matches!(unit, 0x2e | 0x3e))
        {
            self.cursor += 1;
        }
        let tag = CueTag::parse(&self.source[start..self.cursor]);
        let mut classes = Vec::new();
        while self.source.get(self.cursor) == Some(&u16::from(b'.')) {
            self.cursor += 1;
            let start = self.cursor;
            while self
                .source
                .get(self.cursor)
                .is_some_and(|&unit| !tag_whitespace(unit) && !matches!(unit, 0x2e | 0x3e))
            {
                self.cursor += 1;
            }
            let class = &self.source[start..self.cursor];
            if !class.is_empty() {
                if !classes.is_empty() {
                    classes.push(u16::from(b' '));
                }
                classes.extend_from_slice(class);
            }
        }
        let mut annotation = Vec::new();
        if self
            .source
            .get(self.cursor)
            .is_some_and(|&unit| tag_whitespace(unit))
        {
            self.cursor += 1;
            while let Some(&unit) = self.source.get(self.cursor) {
                if unit == u16::from(b'>') {
                    break;
                }
                self.cursor += 1;
                if unit == u16::from(b'&') {
                    consume_reference(self.source, &mut self.cursor, &mut annotation);
                } else {
                    annotation.push(unit);
                }
            }
            // Annotation normalization uses Infra ASCII whitespace, which
            // includes CR but excludes VT, NBSP and other Unicode spaces.
            annotation = annotation
                .split(|&unit| ascii_whitespace(unit))
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(&u16::from(b' '));
        }
        self.consume_tag_end();
        CueToken::Start {
            tag,
            classes,
            annotation,
        }
    }
}

fn tag_whitespace(unit: u16) -> bool {
    matches!(unit, 0x09 | 0x0a | 0x0c | 0x20)
}
fn ascii_whitespace(unit: u16) -> bool {
    tag_whitespace(unit) || unit == 0x0d
}

/// HTML character references in both data and annotation are consumed outside
/// an HTML attribute. Reuse html5ever's complete named-entity and C1 tables.
fn consume_reference(source: &[u16], cursor: &mut usize, output: &mut Vec<u16>) {
    let start = *cursor;
    if source.get(start) == Some(&u16::from(b'#')) {
        let mut end = start + 1;
        let radix = if source
            .get(end)
            .is_some_and(|&unit| matches!(unit, 0x78 | 0x58))
        {
            end += 1;
            16
        } else {
            10
        };
        let digits = end;
        let mut value = 0u32;
        while let Some(digit) = source
            .get(end)
            .and_then(|&unit| char::from_u32(u32::from(unit))?.to_digit(radix))
        {
            value = value.saturating_mul(radix).saturating_add(digit);
            end += 1;
        }
        if end != digits {
            *cursor = end + usize::from(source.get(end) == Some(&u16::from(b';')));
            let value = match value {
                0 => '\u{fffd}',
                0x80..=0x9f => C1_REPLACEMENTS[(value - 0x80) as usize]
                    .unwrap_or(char::from_u32(value).unwrap()),
                _ => char::from_u32(value).unwrap_or('\u{fffd}'),
            };
            output.extend_from_slice(value.encode_utf16(&mut [0; 2]));
            return;
        }
    } else {
        let mut name = String::new();
        let mut matched = None;
        for (offset, &unit) in source[start..].iter().enumerate() {
            if unit > 0x7f {
                break;
            }
            name.push(unit as u8 as char);
            let Some(&(first, second)) = NAMED_ENTITIES.get(name.as_str()) else {
                break;
            };
            if first != 0 {
                matched = Some((start + offset + 1, first, second));
            }
        }
        if let Some((end, first, second)) = matched {
            *cursor = end;
            for point in [first, second].into_iter().filter(|&point| point != 0) {
                output.extend_from_slice(
                    char::from_u32(point)
                        .expect("HTML named entity scalar")
                        .encode_utf16(&mut [0; 2]),
                );
            }
            return;
        }
    }
    output.push(u16::from(b'&'));
}

fn canonical_timestamp(raw: &[u16]) -> Option<String> {
    let value = String::from_utf16(raw).ok()?;
    let parts = value.split(':').collect::<Vec<_>>();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => ("0", *minutes, *seconds),
        [hours, minutes, seconds]
            if !hours.is_empty() && hours.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            (*hours, *minutes, *seconds)
        }
        _ => return None,
    };
    super::parse_vtt_two_digit_range(minutes, 59)?;
    super::parse_vtt_seconds_millis(seconds)?;
    // Serialize decimal components directly, without an f64 round trip.
    let hours = hours.trim_start_matches('0');
    let hours = if hours.is_empty() { "0" } else { hours };
    Some(format!("{hours:0>2}:{minutes}:{seconds}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<CueNode> {
        parse_cue_text(&source.encode_utf16().collect::<Vec<_>>())
    }
    fn text(source: &str) -> Vec<u16> {
        parse(source)
            .into_iter()
            .flat_map(|node| match node.kind {
                CueNodeKind::Text(text) => text,
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn cue_text_decodes_html_references_and_preserves_unknown_suffixes() {
        assert_eq!(String::from_utf16(&text("&AMP;&copy;&notit;&nsubE;&#x1f600;&#128;&#0;&#xD800;&#99999999999999999999;&bogus;&#x;" )).unwrap(),
            "&©¬it;⫅\u{338}😀€\u{fffd}\u{fffd}\u{fffd}&bogus;&#x;");
        assert_eq!(String::from_utf16(&text("&amp;&<c")).unwrap(), "&&");
    }

    #[test]
    fn cue_text_preserves_script_utf16_nul_and_line_breaks() {
        let input = [0xd800, 0, 0x0d, 0x0a, 0xdc00];
        assert_eq!(
            parse_cue_text(&input),
            vec![CueNode {
                parent: None,
                kind: CueNodeKind::Text(input.to_vec())
            }]
        );
        assert!(parse("").is_empty());
    }

    #[test]
    fn cue_text_eof_emits_partial_tags_and_timestamps() {
        for source in ["<c", "<c.", "<c..a."] {
            assert!(matches!(
                &parse(source)[0].kind,
                CueNodeKind::Element {
                    tag: CueTag::Class,
                    ..
                }
            ));
        }
        assert_eq!(
            parse("<00:00.500")[0].kind,
            CueNodeKind::Timestamp("00:00:00.500".into())
        );
        assert!(parse("<").is_empty());
        assert!(parse("< b").is_empty());
    }

    #[test]
    fn cue_text_annotations_decode_then_collapse_only_ascii_whitespace() {
        let nodes = parse("<v.a..b \t Foo&amp;\r\n&#32;Bar\u{a0}\u{b} >text");
        assert_eq!(
            nodes[0].kind,
            CueNodeKind::Element {
                tag: CueTag::Voice,
                classes: "a b".encode_utf16().collect(),
                annotation: "Foo& Bar\u{a0}\u{b}".encode_utf16().collect(),
            }
        );
        assert_eq!(
            parse("<v>")[0].kind,
            CueNodeKind::Element {
                tag: CueTag::Voice,
                classes: vec![],
                annotation: vec![]
            }
        );
        let input = [0x3c, 0x76, 0x2e, 0xd800, 0x20, 0xdc00, 0, 0x3e];
        assert_eq!(
            parse_cue_text(&input)[0].kind,
            CueNodeKind::Element {
                tag: CueTag::Voice,
                classes: vec![0xd800],
                annotation: vec![0xdc00, 0]
            }
        );
    }

    #[test]
    fn cue_text_end_tags_require_exact_names_and_current_node() {
        let nodes = parse("<b>x</b.foo>y</b >z</b>w");
        assert_eq!(
            nodes.iter().map(|node| node.parent).collect::<Vec<_>>(),
            [None, Some(0), Some(0), Some(0), None]
        );
    }

    #[test]
    fn cue_text_ruby_text_requires_immediate_ruby_and_ruby_close_unwinds_it() {
        let nodes = parse("<rt>x<ruby><i><rt>y</i><rt>z</ruby>w");
        assert_eq!(
            nodes.iter().map(|node| node.parent).collect::<Vec<_>>(),
            [None, None, Some(1), Some(2), Some(1), Some(4), None]
        );
        assert!(matches!(
            nodes[4].kind,
            CueNodeKind::Element {
                tag: CueTag::RubyText,
                ..
            }
        ));
    }

    #[test]
    fn cue_text_nested_language_spans_keep_their_own_annotations() {
        let nodes = parse("<lang en><lang fr>x</lang>y</lang>z");
        assert_eq!(
            nodes.iter().map(|node| node.parent).collect::<Vec<_>>(),
            [None, Some(0), Some(1), Some(0), None]
        );
        assert!(
            matches!(&nodes[1].kind, CueNodeKind::Element {tag: CueTag::Language, annotation, ..} if annotation == &"fr".encode_utf16().collect::<Vec<_>>())
        );
    }

    #[test]
    fn cue_text_timestamps_are_exact_and_do_not_round_large_hours() {
        for (source, expected) in [
            ("<1:00:00.500>", "01:00:00.500"),
            ("<000100:00:00.001>", "100:00:00.001"),
            ("<9007199254740993:00:00.999>", "9007199254740993:00:00.999"),
        ] {
            assert_eq!(
                parse(source)[0].kind,
                CueNodeKind::Timestamp(expected.into())
            );
        }
        for source in [
            "<0:00.500>",
            "<60:00.000>",
            "<00:60.000>",
            "<00:00.000x>",
            "<00:00.00>",
            "<00:00.0000>",
        ] {
            assert!(parse(source).is_empty(), "{source}");
        }
    }
}
