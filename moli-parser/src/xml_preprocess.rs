use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    ops::Range,
    rc::Rc,
};

use xmlparser::{
    ElementEnd, EntityDefinition, Token as XmlTokenizerToken, Tokenizer as XmlTokenizer,
};

const MAX_ENTITY_NESTING_DEPTH: usize = 32;
const MAX_ENTITY_EXPANSION_BYTES: usize = 1024 * 1024;

struct ExpansionCandidate {
    range: Range<usize>,
    attribute_quote: Option<u8>,
}

struct Edit {
    range: Range<usize>,
    replacement: String,
}

/// xml5ever rejects every internal DTD subset before it reaches declared
/// general entities. Translate the deliberately narrow subset that Moli can
/// account for into input xml5ever already understands. Unsupported subsets
/// stay untouched so the normal XML parsererror path remains authoritative.
pub(super) fn prepare_xml_for_xml5ever(source: &str) -> Cow<'_, str> {
    try_prepare_xml_for_xml5ever(source).map_or(Cow::Borrowed(source), Cow::Owned)
}

fn try_prepare_xml_for_xml5ever(source: &str) -> Option<String> {
    let mut subset_content_start = None;
    let mut subset_content_end = None;
    let mut subset_token_ranges = Vec::new();
    let mut declarations = HashMap::new();
    let mut candidates = Vec::new();
    let mut inside_subset = false;
    let mut element_depth = 0usize;

    for token in XmlTokenizer::from(source) {
        let token = token.ok()?;
        match token {
            XmlTokenizerToken::DtdStart { span, .. } => {
                if inside_subset || subset_content_start.is_some() {
                    return None;
                }
                inside_subset = true;
                subset_content_start = Some(span.end());
            }
            XmlTokenizerToken::EntityDeclaration {
                name,
                definition,
                span,
            } if inside_subset => {
                subset_token_ranges.push(span.range());
                if is_parameter_entity_declaration(span.as_str())
                    || is_predefined_entity(name.as_str())
                {
                    return None;
                }
                let EntityDefinition::EntityValue(value) = definition else {
                    return None;
                };
                if value.as_str().contains('%') {
                    return None;
                }
                declarations
                    .entry(name.as_str().to_owned())
                    .or_insert_with(|| value.as_str().to_owned());
            }
            XmlTokenizerToken::Comment { span, .. }
            | XmlTokenizerToken::ProcessingInstruction { span, .. }
                if inside_subset =>
            {
                subset_token_ranges.push(span.range());
            }
            XmlTokenizerToken::DtdEnd { span } => {
                if !inside_subset || subset_content_end.is_some() {
                    return None;
                }
                inside_subset = false;
                subset_content_end = Some(span.start());
            }
            XmlTokenizerToken::Attribute { value, .. } if !inside_subset => {
                let quote = value
                    .start()
                    .checked_sub(1)
                    .and_then(|index| source.as_bytes().get(index))
                    .copied()?;
                if !matches!(quote, b'\'' | b'"') {
                    return None;
                }
                candidates.push(ExpansionCandidate {
                    range: value.range(),
                    attribute_quote: Some(quote),
                });
            }
            XmlTokenizerToken::Text { text } if !inside_subset && element_depth > 0 => {
                candidates.push(ExpansionCandidate {
                    range: text.range(),
                    attribute_quote: None,
                });
            }
            XmlTokenizerToken::ElementEnd { end, .. } if !inside_subset => match end {
                ElementEnd::Open => element_depth = element_depth.checked_add(1)?,
                ElementEnd::Close(..) => element_depth = element_depth.saturating_sub(1),
                ElementEnd::Empty => {}
            },
            _ => {}
        }
    }

    if inside_subset {
        return None;
    }
    let content_start = subset_content_start?;
    let content_end = subset_content_end?;
    if content_start > content_end {
        return None;
    }

    subset_token_ranges.sort_unstable_by_key(|range| range.start);
    let mut cursor = content_start;
    for range in &subset_token_ranges {
        if range.start < cursor
            || range.end > content_end
            || !source[cursor..range.start].bytes().all(is_xml_space)
        {
            return None;
        }
        cursor = range.end;
    }
    if !source[cursor..content_end].bytes().all(is_xml_space) {
        return None;
    }

    let mut resolver = EntityResolver::new(declarations);
    let mut inserted_expansion_bytes = 0usize;
    let mut edits = vec![Edit {
        range: content_start.checked_sub(1)?..content_end.checked_add(1)?,
        replacement: String::new(),
    }];

    for candidate in candidates {
        let fragment = &source[candidate.range.clone()];
        let Some(expanded) = resolver.expand_document_fragment(
            fragment,
            candidate.attribute_quote,
            &mut inserted_expansion_bytes,
        )?
        else {
            continue;
        };
        edits.push(Edit {
            range: candidate.range,
            replacement: expanded,
        });
    }

    edits.sort_unstable_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let mut prepared = source.to_owned();
    let mut following_start = source.len();
    for edit in edits {
        if edit.range.end > following_start {
            return None;
        }
        following_start = edit.range.start;
        prepared.replace_range(edit.range, &edit.replacement);
    }
    Some(prepared)
}

fn is_parameter_entity_declaration(span: &str) -> bool {
    span.strip_prefix("<!ENTITY")
        .is_some_and(|remainder| remainder.trim_start().starts_with('%'))
}

fn is_predefined_entity(name: &str) -> bool {
    matches!(name, "amp" | "lt" | "gt" | "apos" | "quot")
}

fn is_xml_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

struct EntityResolver {
    declarations: HashMap<String, String>,
    cache: HashMap<String, Rc<str>>,
    failed: HashSet<String>,
    cached_expansion_bytes: usize,
}

impl EntityResolver {
    fn new(declarations: HashMap<String, String>) -> Self {
        Self {
            declarations,
            cache: HashMap::new(),
            failed: HashSet::new(),
            cached_expansion_bytes: 0,
        }
    }

    fn expand_document_fragment(
        &mut self,
        fragment: &str,
        attribute_quote: Option<u8>,
        inserted_expansion_bytes: &mut usize,
    ) -> Option<Option<String>> {
        let mut output = String::with_capacity(fragment.len());
        let mut cursor = 0;
        let mut changed = false;

        while let Some(relative_ampersand) = fragment[cursor..].find('&') {
            let ampersand = cursor + relative_ampersand;
            output.push_str(&fragment[cursor..ampersand]);
            let Some(relative_semicolon) = fragment[ampersand + 1..].find(';') else {
                output.push_str(&fragment[ampersand..]);
                cursor = fragment.len();
                break;
            };
            let semicolon = ampersand + 1 + relative_semicolon;
            let name = &fragment[ampersand + 1..semicolon];
            if self.declarations.contains_key(name) {
                let replacement = self.resolve(name, &mut Vec::new())?;
                let replacement = escape_attribute_quote(&replacement, attribute_quote);
                *inserted_expansion_bytes =
                    inserted_expansion_bytes.checked_add(replacement.len())?;
                if *inserted_expansion_bytes > MAX_ENTITY_EXPANSION_BYTES {
                    return None;
                }
                output.push_str(&replacement);
                changed = true;
            } else {
                output.push_str(&fragment[ampersand..=semicolon]);
            }
            cursor = semicolon + 1;
        }
        output.push_str(&fragment[cursor..]);

        Some(changed.then_some(output))
    }

    fn resolve(&mut self, name: &str, stack: &mut Vec<String>) -> Option<Rc<str>> {
        if let Some(cached) = self.cache.get(name) {
            return Some(Rc::clone(cached));
        }
        if self.failed.contains(name)
            || stack.len() >= MAX_ENTITY_NESTING_DEPTH
            || stack.iter().any(|ancestor| ancestor == name)
        {
            self.failed.insert(name.to_owned());
            return None;
        }

        let raw = self.declarations.get(name)?.to_owned();
        stack.push(name.to_owned());
        let expanded = self.expand_entity_value(&raw, stack);
        stack.pop();
        let Some(expanded) = expanded else {
            self.failed.insert(name.to_owned());
            return None;
        };

        let cached_expansion_bytes = self.cached_expansion_bytes.checked_add(expanded.len())?;
        if cached_expansion_bytes > MAX_ENTITY_EXPANSION_BYTES {
            self.failed.insert(name.to_owned());
            return None;
        }
        self.cached_expansion_bytes = cached_expansion_bytes;
        let expanded: Rc<str> = Rc::from(expanded);
        self.cache.insert(name.to_owned(), Rc::clone(&expanded));
        Some(expanded)
    }

    fn expand_entity_value(&mut self, value: &str, stack: &mut Vec<String>) -> Option<String> {
        let mut output = String::with_capacity(value.len());
        let mut cursor = 0;

        while let Some(relative_ampersand) = value[cursor..].find('&') {
            let ampersand = cursor + relative_ampersand;
            push_bounded(&mut output, &value[cursor..ampersand])?;
            let Some(relative_semicolon) = value[ampersand + 1..].find(';') else {
                push_bounded(&mut output, &value[ampersand..])?;
                cursor = value.len();
                break;
            };
            let semicolon = ampersand + 1 + relative_semicolon;
            let reference = &value[ampersand + 1..semicolon];
            if self.declarations.contains_key(reference) {
                let replacement = self.resolve(reference, stack)?;
                push_bounded(&mut output, &replacement)?;
            } else {
                push_bounded(&mut output, &value[ampersand..=semicolon])?;
            }
            cursor = semicolon + 1;
        }
        push_bounded(&mut output, &value[cursor..])?;
        Some(output)
    }
}

fn escape_attribute_quote(replacement: &str, quote: Option<u8>) -> Cow<'_, str> {
    match quote {
        Some(b'\'') if replacement.contains('\'') => {
            Cow::Owned(replacement.replace('\'', "&apos;"))
        }
        Some(b'"') if replacement.contains('"') => Cow::Owned(replacement.replace('"', "&quot;")),
        _ => Cow::Borrowed(replacement),
    }
}

fn push_bounded(output: &mut String, fragment: &str) -> Option<()> {
    if output.len().checked_add(fragment.len())? > MAX_ENTITY_EXPANSION_BYTES {
        return None;
    }
    output.push_str(fragment);
    Some(())
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::prepare_xml_for_xml5ever;

    #[test]
    fn expands_general_entities_and_removes_the_internal_subset() {
        let source = concat!(
            "<!DOCTYPE foo [",
            "<!ENTITY a \"A\">",
            "<!ENTITY b \"&a;B\">",
            "<!ENTITY node \"<bar>&b;</bar>\">",
            "<!ENTITY quote \"'\">",
            "]>",
            "<foo attr='&quote;'>&node;</foo>"
        );

        assert_eq!(
            prepare_xml_for_xml5ever(source),
            "<!DOCTYPE foo ><foo attr='&apos;'><bar>AB</bar></foo>"
        );
    }

    #[test]
    fn uses_the_first_duplicate_entity_declaration() {
        let source = "<!DOCTYPE foo [<!ENTITY x \"first\"><!ENTITY x \"second\">]><foo>&x;</foo>";

        assert_eq!(
            prepare_xml_for_xml5ever(source),
            "<!DOCTYPE foo ><foo>first</foo>"
        );
    }

    #[test]
    fn leaves_unsupported_or_recursive_subsets_for_the_xml_parser() {
        for source in [
            "<!DOCTYPE foo [<!ELEMENT foo (#PCDATA)>]><foo/>",
            "<!DOCTYPE foo [<!ENTITY % pe \"value\">]><foo/>",
            "<!DOCTYPE foo [<!ENTITY external SYSTEM \"entity.xml\">]><foo/>",
            "<!DOCTYPE foo [<!ENTITY loop \"&loop;\">]><foo>&loop;</foo>",
        ] {
            assert!(matches!(
                prepare_xml_for_xml5ever(source),
                Cow::Borrowed(unchanged) if unchanged == source
            ));
        }
    }
}
