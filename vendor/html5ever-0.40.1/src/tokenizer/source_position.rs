// SPDX-License-Identifier: MIT OR Apache-2.0

/// Cursor in the original input, before character-reference expansion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SourcePosition {
    pub(super) line: u64,
    pub(super) column: u64,
    after_cr: bool,
}

impl Default for SourcePosition {
    fn default() -> Self {
        Self {
            line: 1,
            column: 1,
            after_cr: false,
        }
    }
}

impl SourcePosition {
    pub(super) fn advance_char(&mut self, c: char) {
        match c {
            '\r' => {
                self.line = self.line.saturating_add(1);
                self.column = 1;
            }
            '\n' => {
                if !self.after_cr {
                    self.line = self.line.saturating_add(1);
                }
                self.column = 1;
            }
            _ => self.column = self.column.saturating_add(c.len_utf16() as u64),
        }
        self.after_cr = c == '\r';
    }

    /// Reuse the SIMD scan's newline count; its consumed prefix has no CR.
    pub(super) fn advance_data_chunk(&mut self, text: &str, newlines: u64) {
        debug_assert!(!text.contains('\r'));
        let folded_lf = u64::from(self.after_cr && text.starts_with('\n'));
        self.line = self.line.saturating_add(newlines - folded_lf);
        if let Some(last_lf) = text.rfind('\n') {
            self.column = 1 + utf16_len(&text[last_lf + 1..]);
        } else {
            self.column = self.column.saturating_add(utf16_len(text));
        }
        self.after_cr = false;
    }
}

fn utf16_len(text: &str) -> u64 {
    if text.is_ascii() {
        text.len() as u64
    } else {
        text.encode_utf16().count() as u64
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::SourcePosition;
    use crate::tendril::StrTendril;
    use crate::tokenizer::{
        BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
    };

    #[derive(Debug, PartialEq, Eq)]
    struct PositionedTag {
        name: String,
        kind: TagKind,
        line: u64,
        column: u64,
    }

    #[derive(Default)]
    struct Sink {
        tags: RefCell<Vec<PositionedTag>>,
        tokens: RefCell<Vec<Token>>,
    }

    impl TokenSink for Sink {
        type Handle = ();

        fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
            self.tokens.borrow_mut().push(token);
            TokenSinkResult::Continue
        }

        fn process_tag_token_with_position(
            &self,
            tag: Tag,
            line: u64,
            column: u64,
        ) -> TokenSinkResult<()> {
            self.tags.borrow_mut().push(PositionedTag {
                name: tag.name.to_string(),
                kind: tag.kind,
                line,
                column,
            });
            self.process_token(Token::TagToken(tag), line)
        }
    }

    fn tokenize(
        chunks: &[&str],
        opts: TokenizerOpts,
    ) -> (Vec<PositionedTag>, Vec<Token>, SourcePosition) {
        let tokenizer = Tokenizer::new(Sink::default(), opts);
        let input = BufferQueue::default();
        for chunk in chunks {
            input.push_back(StrTendril::from(*chunk));
            assert!(matches!(
                tokenizer.feed(&input),
                markup5ever::TokenizerResult::Done
            ));
        }
        tokenizer.end();
        (
            tokenizer.sink.tags.take(),
            tokenizer.sink.tokens.take(),
            tokenizer.source_position.get(),
        )
    }

    // Independently calculate the expected location from literal source text.
    fn position_after(source: &str) -> (u64, u64) {
        let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
        (
            (normalized.bytes().filter(|&c| c == b'\n').count() + 1) as u64,
            (normalized
                .rsplit('\n')
                .next()
                .unwrap()
                .encode_utf16()
                .count()
                + 1) as u64,
        )
    }

    fn assert_script_position(chunks: &[&str], expected: (u64, u64), exact_errors: bool) {
        let (tags, _, _) = tokenize(
            chunks,
            TokenizerOpts {
                track_tag_positions: true,
                exact_errors,
                ..TokenizerOpts::default()
            },
        );
        let positions: Vec<_> = tags
            .into_iter()
            .filter(|tag| tag.name == "script" && tag.kind == TagKind::StartTag)
            .map(|tag| (tag.line, tag.column))
            .collect();
        assert_eq!(
            positions,
            [expected],
            "chunks={chunks:?}, exact_errors={exact_errors}"
        );
    }

    #[test]
    fn completed_tags_preserve_original_utf16_positions_at_every_input_split() {
        for (prefix, tag) in [
            ("", "<script>"),
            ("  中文😀🧪", "<script data-x='😀>é'>"),
            ("one\rtwo\nthree\r\nfour", "<script\r\n data-x='😀'>"),
            ("&amp;&NotEqualTilde;&Afr;&#10;&#x1f600;", "<script>"),
            ("&#xQ;&#Q;&#;&NotARealReference;&notit;", "<script>"),
            ("&not\r\n&unknown\r&#x\n", "<script>"),
            (
                "<div title='&notit;&amp=foo;&#xQ;'></div>",
                "<script data-x='&not\r\n'>",
            ),
            (
                "<!DOCTYPE html PUBLIC 'identifier'><!bogus><!-- 😀\r\n -->",
                "<script>",
            ),
            ("<svg><![CDATA[a\r\n😀]]>", "<script data-x='value'>"),
            ("<p>\u{feff}text</p>", "<script>"),
            ("<x a='\0'>\u{fdd0}é</x>", "<script duplicate duplicate>"),
        ] {
            let before = format!("{prefix}{tag}");
            let expected = position_after(&before);
            let source = format!("{before}body</script>");
            for exact_errors in [false, true] {
                for split in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
                    assert_script_position(
                        &[&source[..split], &source[split..]],
                        expected,
                        exact_errors,
                    );
                }
                let chunks: Vec<_> = source
                    .char_indices()
                    .map(|(i, c)| &source[i..i + c.len_utf8()])
                    .collect();
                assert_script_position(&chunks, expected, exact_errors);
            }
        }
    }

    #[test]
    fn simd_text_prefixes_preserve_newlines_and_utf16_tail_columns() {
        for prefix in [
            "a".repeat(8192),
            format!("{}\n😀tail", "a".repeat(8192)),
            format!("{}\r\n{}\n尾😀", "é".repeat(2048), "x".repeat(8192)),
        ] {
            let before = format!("{prefix}<script>");
            let source = format!("{before}body</script>");
            for split in [0, 1, 16, 32, prefix.len(), source.len()] {
                if source.is_char_boundary(split) {
                    assert_script_position(
                        &[&source[..split], &source[split..]],
                        position_after(&before),
                        false,
                    );
                }
            }
        }
    }

    #[test]
    fn eof_character_reference_rewinds_consume_each_original_character_once() {
        for source in [
            "&",
            "&#",
            "&#x",
            "&#X",
            "&#12",
            "&noti",
            "&unknown",
            "&bogus\r\n",
            "&amp\r",
        ] {
            for exact_errors in [false, true] {
                for split in 0..=source.len() {
                    let (_, _, position) = tokenize(
                        &[&source[..split], &source[split..]],
                        TokenizerOpts {
                            track_tag_positions: true,
                            exact_errors,
                            ..TokenizerOpts::default()
                        },
                    );
                    assert_eq!(
                        (position.line, position.column),
                        position_after(source),
                        "source={source:?}, split={split}"
                    );
                }
            }
        }
    }

    #[test]
    fn only_the_initial_bom_is_discarded_across_input_chunks() {
        for discard_bom in [false, true] {
            let (tags, tokens, _) = tokenize(
                &["", "\u{feff}", "a", "\u{feff}", "<script></script>"],
                TokenizerOpts {
                    track_tag_positions: true,
                    discard_bom,
                    ..TokenizerOpts::default()
                },
            );
            let text: String = tokens
                .iter()
                .filter_map(|token| match token {
                    Token::CharacterTokens(text) => Some(text.as_ref()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                text,
                if discard_bom {
                    "a\u{feff}"
                } else {
                    "\u{feff}a\u{feff}"
                }
            );
            assert_eq!(
                (tags[0].line, tags[0].column),
                (1, if discard_bom { 11 } else { 12 })
            );
        }
    }

    fn merge_character_tokens(tokens: Vec<Token>) -> Vec<Token> {
        let mut result = Vec::new();
        for token in tokens {
            match token {
                Token::CharacterTokens(text) => match result.last_mut() {
                    Some(Token::CharacterTokens(previous)) => previous.push_tendril(&text),
                    _ => result.push(Token::CharacterTokens(text)),
                },
                token => result.push(token),
            }
        }
        result
    }

    #[test]
    fn tracking_is_opt_in_and_does_not_change_token_contents() {
        let chunks = [
            "<!DOCTYPE html>é&not\r",
            "\n<script a='&#xQ;😀'>body</script>",
        ];
        for exact_errors in [false, true] {
            let (legacy_tags, legacy, _) = tokenize(
                &chunks,
                TokenizerOpts {
                    exact_errors,
                    ..TokenizerOpts::default()
                },
            );
            let (tags, tracked, _) = tokenize(
                &chunks,
                TokenizerOpts {
                    track_tag_positions: true,
                    exact_errors,
                    ..TokenizerOpts::default()
                },
            );
            assert!(legacy_tags.is_empty());
            assert_eq!(tags.len(), 2);
            assert_eq!(
                merge_character_tokens(legacy),
                merge_character_tokens(tracked)
            );
        }
    }

    #[test]
    fn default_token_sink_hook_accepts_tracked_tags() {
        struct LegacySink(RefCell<Vec<Token>>);
        impl TokenSink for LegacySink {
            type Handle = ();
            fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
                self.0.borrow_mut().push(token);
                TokenSinkResult::Continue
            }
        }
        let tokenizer = Tokenizer::new(
            LegacySink(RefCell::new(Vec::new())),
            TokenizerOpts {
                track_tag_positions: true,
                ..TokenizerOpts::default()
            },
        );
        let input = BufferQueue::default();
        input.push_back(StrTendril::from("<a>text</a>"));
        let _ = tokenizer.feed(&input);
        tokenizer.end();
        assert_eq!(
            tokenizer
                .sink
                .0
                .take()
                .into_iter()
                .filter(|t| matches!(t, Token::TagToken(_)))
                .count(),
            2
        );
    }

    #[test]
    fn profiled_tokens_use_the_same_tag_position_hook() {
        let (tags, _, _) = tokenize(
            &["😀\r", "\n<script></script>"],
            TokenizerOpts {
                track_tag_positions: true,
                profile: true,
                ..TokenizerOpts::default()
            },
        );
        assert_eq!((tags[0].line, tags[0].column), (2, 9));
    }
}
