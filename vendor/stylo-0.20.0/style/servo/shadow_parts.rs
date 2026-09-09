/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::derives::*;
use crate::values::AtomIdent;
use crate::Atom;

type Mapping<'a> = (&'a str, &'a str);

#[derive(Clone, Debug, MallocSizeOf)]
pub struct ShadowParts {
    // FIXME: Consider a smarter data structure for this.
    // Gecko uses a hashtable in both directions:
    // https://searchfox.org/mozilla-central/rev/5d4178378f84c7130ccb8ac1723d33e380d7f7d7/layout/style/ShadowParts.h
    mappings: Vec<(Atom, Atom)>,
}

/// <https://drafts.csswg.org/css-shadow-parts/#parsing-mapping>
///
/// Returns `None` in the failure case.
pub fn parse_part_mapping(input: &str) -> Option<Mapping<'_>> {
    // Step 1. Let input be the string being parsed.
    // Step 2. Let position be a pointer into input, initially pointing at the start of the string.
    // NOTE: We don't need an explicit position, we just drop stuff from the input

    // Step 3. Collect a sequence of code points that are space characters
    let input = input.trim_start_matches(|c| c == ' ');

    // Step 4. Collect a sequence of code points that are not space characters or U+003A COLON characters,
    // and let first token be the result.
    let space_or_colon_position = input
        .char_indices()
        .find(|(_, c)| matches!(c, ' ' | ':'))
        .map(|(index, _)| index)
        .unwrap_or(input.len());
    let (first_token, input) = input.split_at(space_or_colon_position);

    // Step 5. If first token is empty then return error.
    if first_token.is_empty() {
        return None;
    }

    // Step 6. Collect a sequence of code points that are space characters.
    let input = input.trim_start_matches(|c| c == ' ');

    // Step 7. If the end of the input has been reached, return the tuple (first token, first token)
    if input.is_empty() {
        return Some((first_token, first_token));
    }

    // Step 8. If character at position is not a U+003A COLON character, return error.
    // Step 9. Consume the U+003A COLON character.
    let Some(input) = input.strip_prefix(':') else {
        return None;
    };

    // Step 10. Collect a sequence of code points that are space characters.
    let input = input.trim_start_matches(|c| c == ' ');

    // Step 11. Collect a sequence of code points that are not space characters or U+003A COLON characters.
    // and let second token be the result.
    let space_or_colon_position = input
        .char_indices()
        .find(|(_, c)| matches!(c, ' ' | ':'))
        .map(|(index, _)| index)
        .unwrap_or(input.len());
    let (second_token, input) = input.split_at(space_or_colon_position);

    // Step 12. If second token is empty then return error.
    if second_token.is_empty() {
        return None;
    }

    // Step 13. Collect a sequence of code points that are space characters.
    let input = input.trim_start_matches(|c| c == ' ');

    // Step 14. If position is not past the end of input then return error.
    if !input.is_empty() {
        return None;
    }

    // Step 14. Return the tuple (first token, second token).
    Some((first_token, second_token))
}

/// <https://drafts.csswg.org/css-shadow-parts/#parsing-mapping-list>
fn parse_mapping_list(input: &str) -> impl Iterator<Item = Mapping<'_>> {
    // Step 1. Let input be the string being parsed.
    // Step 2. Split the string input on commas. Let unparsed mappings be the resulting list of strings.
    let unparsed_mappings = input.split(',');

    // Step 3. Let mappings be an initially empty list of tuples of tokens.
    // This list will be the result of this algorithm.
    // NOTE: We return an iterator here - it is up to the caller to turn it into a list

    // Step 4. For each string unparsed mapping in unparsed mappings, run the following substeps:
    unparsed_mappings.filter_map(|unparsed_mapping| {
        // Step 4.1 If unparsed mapping is empty or contains only space characters,
        // continue to the next iteration of the loop.
        if unparsed_mapping.chars().all(|c| c == ' ') {
            return None;
        }

        // Step 4.2 Let mapping be the result of parsing unparsed mapping using the rules for parsing part mappings.
        // Step 4.3 If mapping is an error then continue to the next iteration of the loop.
        // This allows clients to skip over new syntax that is not understood.
        // Step 4.4 Append mapping to mappings.
        parse_part_mapping(unparsed_mapping)
    })
}

impl ShadowParts {
    pub fn parse(input: &str) -> Self {
        Self {
            mappings: parse_mapping_list(input)
                .map(|(first, second)| (first.into(), second.into()))
                .collect(),
        }
    }

    /// Call the provided callback for each exported part with the given name.
    pub fn for_each_exported_part<F>(&self, name: &Atom, mut callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        for (from, to) in &self.mappings {
            if from == name {
                callback(AtomIdent::cast(to));
            }
        }
    }

    pub fn imported_part(&self, name: &Atom) -> Option<&Atom> {
        self.mappings
            .iter()
            .find(|(_, to)| to == name)
            .map(|(from, _)| from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_mapping() {
        assert_eq!(
            parse_part_mapping("foo"),
            Some(("foo", "foo")),
            "Single token"
        );
        assert_eq!(
            parse_part_mapping("  foo"),
            Some(("foo", "foo")),
            "Single token with leading whitespace"
        );
        assert_eq!(
            parse_part_mapping("foo "),
            Some(("foo", "foo")),
            "Single token with trailing whitespace"
        );
        assert_eq!(
            parse_part_mapping("foo:bar"),
            Some(("foo", "bar")),
            "Two tokens"
        );
        assert_eq!(
            parse_part_mapping("foo:bar "),
            Some(("foo", "bar")),
            "Two tokens with trailing whitespace"
        );
        assert_eq!(
            parse_part_mapping("🦀:🚀"),
            Some(("🦀", "🚀")),
            "Two tokens consisting of non-ascii characters"
        );
    }

    #[test]
    fn reject_invalid_mapping() {
        assert!(parse_part_mapping("").is_none(), "Empty input");
        assert!(parse_part_mapping("    ").is_none(), "Only whitespace");
        assert!(parse_part_mapping("foo bar").is_none(), "Missing colon");
        assert!(parse_part_mapping(":bar").is_none(), "Empty first token");
        assert!(parse_part_mapping("foo:").is_none(), "Empty second token");
        assert!(
            parse_part_mapping("foo:bar baz").is_none(),
            "Trailing input"
        );
    }

    #[test]
    fn parse_valid_mapping_list() {
        let mut mappings = parse_mapping_list("foo: bar, totally invalid mapping,,");

        // "foo: bar" is a valid mapping
        assert_eq!(
            mappings.next(),
            Some(("foo", "bar")),
            "First mapping should be in the list"
        );
        // "totally invalid mapping" is not a valid mapping and should be ignored
        // "" is not valid (and consists of nothing but whitespace), so it should be ignored
        assert!(mappings.next().is_none(), "No more mappings should exist");
    }
}
