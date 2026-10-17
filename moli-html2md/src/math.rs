/// Locate a complete author-supplied TeX span before ordinary Markdown escaping.
/// Dollar boundaries follow the usual non-space/non-digit rules, so prices do
/// not consume the prose between two currency amounts.
pub(crate) fn spans(text: &str) -> Vec<(usize, usize)> {
    let mut steps = 0;
    spans_counted(text, &mut steps)
}

fn spans_counted(text: &str, steps: &mut usize) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut backslashes = 0;
    let mut dollar = None;
    let mut double_dollar = None;
    let mut paren = None;
    let mut bracket = None;
    let mut candidates = Vec::new();
    while index < bytes.len() {
        *steps += 1;
        if bytes[index] == b'$' && backslashes % 2 == 0 {
            if bytes.get(index + 1) == Some(&b'$') {
                if let Some(start) = double_dollar.take() {
                    accept_candidate(text, start, index + 2, 2, false, &mut candidates);
                } else {
                    double_dollar = Some(index);
                }
                index += 2;
                backslashes = 0;
                continue;
            }
            if let Some(start) = dollar.take() {
                accept_candidate(text, start, index + 1, 1, true, &mut candidates);
            } else if text[index + 1..]
                .chars()
                .next()
                .is_some_and(|ch| !ch.is_whitespace())
            {
                dollar = Some(index);
            }
            index += 1;
            backslashes = 0;
            continue;
        }
        if bytes[index] == b'\\' && backslashes % 2 == 0 {
            match bytes.get(index + 1) {
                Some(b'(') => {
                    paren.get_or_insert(index);
                }
                Some(b'[') => {
                    bracket.get_or_insert(index);
                }
                Some(b')') => {
                    if let Some(start) = paren.take() {
                        accept_candidate(text, start, index + 2, 2, false, &mut candidates);
                    }
                }
                Some(b']') => {
                    if let Some(start) = bracket.take() {
                        accept_candidate(text, start, index + 2, 2, false, &mut candidates);
                    }
                }
                _ => {}
            }
            if matches!(bytes.get(index + 1), Some(b'(' | b'[' | b')' | b']')) {
                index += 2;
                backslashes = 0;
                continue;
            }
        }
        if bytes[index] == b'\\' {
            backslashes += 1;
        } else {
            backslashes = 0;
        }
        index += 1;
    }
    candidates.sort_unstable_by_key(|candidate| candidate.0);
    let mut end = 0;
    candidates.retain(|candidate| {
        let keep = candidate.0 >= end;
        if keep {
            end = candidate.1;
        }
        keep
    });
    candidates
}

fn accept_candidate(
    text: &str,
    start: usize,
    end: usize,
    delimiter: usize,
    single_dollar: bool,
    candidates: &mut Vec<(usize, usize)>,
) {
    let content_start = start + delimiter;
    let content_end = end - delimiter;
    if content_start == content_end
        || single_dollar
            && (text[..content_end]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
                || text.as_bytes().get(end).is_some_and(u8::is_ascii_digit))
    {
        return;
    }
    let content = &text[content_start..content_end];
    if content.as_bytes().windows(2).any(|pair| {
        pair[0] == b'<' && (pair[1].is_ascii_alphabetic() || matches!(pair[1], b'/' | b'!' | b'?'))
    }) || contains_markdown_resource(content)
    {
        return;
    }
    candidates.push((start, end));
}

fn contains_markdown_resource(text: &str) -> bool {
    text.contains("](") || text.contains("][") || text.contains("![[")
}

#[cfg(test)]
mod tests {
    use super::spans_counted;

    #[test]
    fn math_scan_work_is_linear_for_backslashes_and_unclosed_delimiters() {
        for text in ["\\\\".repeat(8_192), "\\(x ".repeat(4_096)] {
            let mut steps = 0;
            let _ = spans_counted(&text, &mut steps);
            assert!(
                steps <= text.len(),
                "{steps} steps for {} bytes",
                text.len()
            );
        }
    }

    #[test]
    fn mixed_unclosed_and_valid_math_is_scanned_once() {
        for size in [256, 512, 1024, 2048] {
            let text = r"\( $x_i$ ".repeat(size);
            let mut steps = 0;
            let spans = spans_counted(&text, &mut steps);
            assert_eq!(spans.len(), size);
            assert!(
                steps <= text.len(),
                "{steps} steps for {} bytes",
                text.len()
            );
        }
    }
}
